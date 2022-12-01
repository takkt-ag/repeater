// Copyright 2023 KAISER+KRAFT EUROPA GmbH
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

mod de;
mod ser;

use std::{
    io::{
        self,
        Write,
    },
    path::{
        Path,
        PathBuf,
    },
    sync::Arc,
    time::Instant,
};

use anyhow::Result;
use clap::{
    Args,
    Parser,
    Subcommand,
};
use hifitime::{
    Duration,
    Epoch,
};
use indicatif::{
    ProgressBar,
    ProgressStyle,
};
use reqwest::{
    Client,
    Request,
};
use serde::{
    Deserialize,
    Serialize,
};
use tracing_subscriber::{
    filter::{
        EnvFilter,
        LevelFilter,
    },
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

#[derive(Debug, Deserialize)]
struct AccessLogRecord {
    #[serde(
        rename = "@timestamp",
        deserialize_with = "crate::de::kibana_timestamp_as_epoch"
    )]
    timestamp: Epoch,
    path: String,
    #[serde(rename = "params")]
    parameters: String,
    #[serde(rename = "target_processing_time")]
    required_time: f64,
}

#[derive(Debug)]
struct RequestWithOffset {
    offset: Duration,
    request: Request,
    record: AccessLogRecord,
}

impl AccessLogRecord {
    fn records_from_path<P: AsRef<Path>>(path: P) -> Result<Vec<AccessLogRecord>> {
        let reader = csv::Reader::from_path(path)?;
        let mut records = reader
            .into_deserialize::<AccessLogRecord>()
            .map(|row| row.map_err(Into::into))
            .collect::<Result<Vec<_>>>()?;
        records.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());

        Ok(records)
    }

    fn requests_from_path<P: AsRef<Path>>(
        path: P,
        client: &Client,
        scheme_and_host: &str,
    ) -> Result<Vec<RequestWithOffset>> {
        let mut first_timestamp = None;
        Self::records_from_path(path)?
            .into_iter()
            .map(|record| {
                let offset = first_timestamp
                    .map(|first_timestamp| record.timestamp - first_timestamp)
                    .unwrap_or_default();
                first_timestamp.get_or_insert(record.timestamp);

                client
                    .get(format!(
                        "{}{}{}",
                        scheme_and_host, record.path, record.parameters
                    ))
                    .build()
                    .map(|request| RequestWithOffset {
                        offset,
                        request,
                        record,
                    })
                    .map_err(Into::into)
            })
            .collect::<Result<Vec<_>>>()
    }
}

#[derive(Debug, Parser)]
#[command(author, version, about, propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Print(Print),
    Run(Run),
}

/// Parse a provided Kibana CSV-export containing at least the fields `@timestamp', `path1 and `params`, and print every
/// row as a separate, structured line, in order (by timestamp).
#[derive(Debug, Args)]
struct Print {
    /// CSV-file to parse and print.
    input_file: PathBuf,
}

impl Print {
    fn run(&self) -> Result<()> {
        let mut last_timestamp = None;
        for record in AccessLogRecord::records_from_path(&self.input_file)? {
            let offset = match last_timestamp {
                Some(last_timestamp) => record.timestamp - last_timestamp,
                None => Duration::from_seconds(0.0),
            };

            println!(
                "{} {} {}{}",
                record.timestamp,
                format_args!("+{:>12}", offset),
                record.path,
                record.parameters
            );
            last_timestamp = Some(record.timestamp);
        }

        Ok(())
    }
}

/// GET provided URLs again, with accurate relative timing.
///
/// Parses the provided Kibana CSV-export and runs the discovered requests, with accurate relative timing, against the
/// provided host.
#[derive(Debug, Args)]
struct Run {
    /// Scheme and host to run the GET-requests against.
    ///
    /// Example: `https://my-alternative-service.internal`.
    #[arg(short, long)]
    scheme_and_host: String,
    /// CSV-file to parse and GET-again.
    input_file: PathBuf,
}

impl Run {
    async fn run(&self) -> Result<()> {
        let client = Arc::new(Client::new());
        let requests =
            AccessLogRecord::requests_from_path(&self.input_file, &client, &self.scheme_and_host)?;
        if requests.is_empty() {
            anyhow::bail!("No records in provided file");
        }
        let last = requests
            .last()
            .expect("Vec should be non-empty at this point!");
        let minimum_expected_runtime = last.offset;

        tracing::info!(
            "Starting to execute {} requests, minimum runtime is: {}",
            requests.len(),
            minimum_expected_runtime
        );

        let pb = ProgressBar::new(requests.len() as u64).with_style(ProgressStyle::with_template(
            "[{elapsed}] {wide_bar} {pos:>7}/{len:7}",
        )?);
        let join_handles = requests
            .into_iter()
            .map(|request_with_offset| {
                tokio::spawn({
                    let client = client.clone();
                    let pb = pb.clone();
                    async move {
                        let result = Self::get(&*client, request_with_offset).await;
                        pb.inc(1);
                        result
                    }
                })
            })
            .collect::<Vec<_>>();

        let mut stdout = io::stdout().lock();
        for response_details in futures::future::join_all(join_handles)
            .await
            .into_iter()
            .map(|join_handle| join_handle.map_err(Into::into))
            .collect::<Result<Vec<_>>>()?
        {
            match response_details {
                Ok(response_details) => {
                    serde_json::to_writer(&mut stdout, &response_details)?;
                    write!(stdout, "\n")?;
                }
                Err(err) => eprintln!("{}", err),
            }
        }
        Ok(())
    }

    async fn get(
        client: &Client,
        RequestWithOffset {
            request,
            offset,
            record,
        }: RequestWithOffset,
    ) -> Result<ResponseDetails> {
        tokio::time::sleep(offset.into()).await;
        let url = request.url().as_str().to_owned();
        let start = Instant::now();
        let response = client.execute(request).await?.error_for_status()?;
        let required_time = Duration::from(start.elapsed());
        tracing::debug!(
            "Request={}..., waited_for={}, status={}, required_time={}",
            &url[..64],
            offset,
            response.status(),
            required_time
        );
        let original_time = record.required_time;
        let change_percentage =
            ((required_time.to_seconds() - original_time) / original_time) * 100.;
        Ok(ResponseDetails {
            url,
            status: response.status(),
            required_time,
            original_time,
            change_percentage,
        })
    }
}

#[derive(Debug, Serialize)]
struct ResponseDetails {
    url: String,
    #[serde(serialize_with = "crate::ser::statuscode_as_u16")]
    status: reqwest::StatusCode,
    #[serde(serialize_with = "crate::ser::duration_to_seconds")]
    required_time: Duration,
    original_time: f64,
    change_percentage: f64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Print(args) => args.run(),
        Commands::Run(args) => args.run().await,
    }
}
