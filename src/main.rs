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
    fs::File,
    io::{
        self,
        BufReader,
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

#[derive(Debug, Deserialize)]
struct JsonAccessLogRecord {
    #[serde(rename = "_source")]
    source: AccessLogRecord,
}

impl From<JsonAccessLogRecord> for AccessLogRecord {
    fn from(json_record: JsonAccessLogRecord) -> Self {
        json_record.source
    }
}

#[derive(Debug)]
struct RequestWithOffset {
    offset: Duration,
    request: Request,
    record: AccessLogRecord,
}

impl AccessLogRecord {
    fn records_from_path<P: AsRef<Path>>(path: P) -> Result<Vec<AccessLogRecord>> {
        let mut records = match path.as_ref().extension().and_then(|ext| ext.to_str()) {
            Some("csv") => Self::records_from_csv_path(path),
            Some("json") => Self::records_from_json_path(path),
            Some(ext) => anyhow::bail!("Unknown file extension: {}", ext),
            None => anyhow::bail!("Can't determine file-type"),
        }?;
        records.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());

        Ok(records)
    }

    fn records_from_csv_path<P: AsRef<Path>>(path: P) -> Result<Vec<AccessLogRecord>> {
        let reader = csv::Reader::from_path(path)?;
        reader
            .into_deserialize::<AccessLogRecord>()
            .map(|row| row.map_err(Into::into))
            .collect::<Result<Vec<_>>>()
    }

    fn records_from_json_path<P: AsRef<Path>>(path: P) -> Result<Vec<AccessLogRecord>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        serde_json::Deserializer::from_reader(reader)
            .into_iter::<JsonAccessLogRecord>()
            .map(|item| item.map(Into::<AccessLogRecord>::into).map_err(Into::into))
            .collect()
    }

    fn requests_from_path<P: AsRef<Path>>(
        path: P,
        client: &Client,
        scheme_and_host: &str,
        time_factor: Option<f64>,
    ) -> Result<Vec<RequestWithOffset>> {
        let mut first_timestamp = None;
        Self::records_from_path(path)?
            .into_iter()
            .map(|record| {
                let time_factor = time_factor.unwrap_or(1f64);
                let offset = first_timestamp
                    .map(|first_timestamp| record.timestamp - first_timestamp)
                    .unwrap_or_default()
                    * time_factor;
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

/// Parse the provided file containing at least the fields `@timestamp', `path` and `params`, and print every
/// row as a separate, structured line, in order (by timestamp).
#[derive(Debug, Args)]
struct Print {
    /// File to parse and print.
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
/// Parses the provided file and runs the discovered requests, with accurate relative timing, against the
/// provided host.
#[derive(Debug, Args)]
struct Run {
    /// Scheme and host to run the GET-requests against.
    ///
    /// Example: `https://my-alternative-service.internal`.
    #[arg(short, long)]
    scheme_and_host: String,
    /// File to parse and GET-again.
    input_file: PathBuf,
    /// Factor in which the requests should be fulfilled.
    ///
    /// 0.5 will mean the requests finish in half the time (double the load), whereas 2.0 would mean the requests finish
    /// in double the time (half the load).
    #[arg(long)]
    time_factor: Option<f64>,
}

impl Run {
    async fn run(&self) -> Result<()> {
        let client = Arc::new(Client::new());
        let requests = AccessLogRecord::requests_from_path(
            &self.input_file,
            &client,
            &self.scheme_and_host,
            self.time_factor,
        )?;
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

        let mut join_set = tokio::task::JoinSet::new();
        for request_with_offset in requests {
            join_set.spawn({
                let client = client.clone();
                let pb = pb.clone();
                async move {
                    let result = Self::get(&client, request_with_offset).await;
                    pb.inc(1);
                    result
                }
            });
        }

        let mut responses: Vec<Result<ResponseDetails>> = Vec::new();
        let clean_exit = loop {
            tokio::select! {
                response = join_set.join_next() => {
                    match response {
                        Some(response) => responses.push(response?),
                        None => {
                            break true
                        }
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    break false
                }
            }
        };

        let mut stdout = io::stdout().lock();
        for response_details in responses {
            match response_details {
                Ok(response_details) => {
                    serde_json::to_writer(&mut stdout, &response_details)?;
                    writeln!(stdout)?;
                }
                Err(err) => eprintln!("{}", err),
            }
        }

        if clean_exit {
            Ok(())
        } else {
            anyhow::bail!("Aborted with CTRL-C")
        }
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

#[tokio::main(flavor = "multi_thread", worker_threads = 64)]
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
