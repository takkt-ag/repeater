# repeater

A command-line tool, `r7`, to parse and then repeat GET-requests of an access-log, against a different host with precise relative timing.

`r7` can also print all requests it parsed instead of repeating them.

While repeating requests, `r7` will continuously keep track of when a request should be performed, relative to when you started the replay.
By default `r7` will attempt to replay the requests as close to the original timing as possible.
If you want to replay the requests slower or faster, for example to not overload a test system or to simulate higher load, you can apply a time-factor to the replay.

> [!IMPORTANT]
> This project is still in early development.
> Although we have used it to replay tens of thousands of `GET`-requests, there is a chance that there are bugs that could lead to incorrect requests being sent or the replay not being time-accurate.
>
> Additionally, the documentation is still in its early stages, and the compatibility with input file formats is still limited.

## Building

To build the project, you need to have Rust installed.
You can install Rust by following the instructions at <https://www.rust-lang.org/tools/install>.

Once you have Rust installed, you can build the project by running the following command:

```sh
cargo build --release
```

The built binary will be available at `target/release/r7`.

## Usage

An `r7` command has the following structure:

```sh
$ r7 <COMMAND> [options and arguments]
```

To print the requests `r7` is able to parse from a given file, which can be a nice sanity check, you can run:

```sh
$ r7 print <INPUT_FILE>
```

To actually run a replay, you can use the `run` command:

```text
Usage: r7 run [OPTIONS] --scheme-and-host <SCHEME_AND_HOST> <INPUT_FILE>

Arguments:
  <INPUT_FILE>
          File to parse and GET-again

Options:
  -s, --scheme-and-host <SCHEME_AND_HOST>
          Scheme and host to run the GET-requests against.
          
          Example: `https://my-alternative-service.internal`.

      --time-factor <TIME_FACTOR>
          Factor in which the requests should be fulfilled.
          
          0.5 will mean the requests finish in half the time (double the load), whereas 2.0 would
          mean the requests finish in double the time (half the load).
```

To view the help documentation, use one of the following commands:

```sh
$ r7 --help
$ r7 <command> --help
```

To get the version of the tool, use the following command:

```sh
$ r7 --version
```

### Input file format

`r7` has been written under the pretense of having exported AWS Application Loadbalancer logs exported to S3 and then imported into an OpenSearch index.
Therefore, the file format is supposed to be an export from an ElasticSearch or OpenSearch index, either as CSV or JSONL.

The following fields are expected to be present in the provided file:

* `@timestamp` (string): The timestamp in ISO8601 format when the request happened.
* `path` (string): The URL-path of the request.
* `params` (string): The query-parameters of the request.
* `target_processing_time` (number): The time the original request took to process, in seconds (can be fractional).

  This field is used to determine how the replayed request performed in comparison to the original.

All requests in this file will be repeated as `GET`-requests against the specified host and scheme.

> [!WARNING]
> Other request methods are not supported.
> `r7` will send all requests as `GET`-requests, which you probably don't want for requests that weren't `GET`s in the first place.

#### JSONL/JSON-ND

You can provide the requests as a file that contains new-line delimited JSON objects, where each object follows the following structure:

```json
{
  "_source": {
    "@timestamp": "",
    "path": "",
    "params": "",
    "target_processing_time": 0.0
  }
}
```

The JSON-objects may contain additional fields, but only the fields mentioned above will be used.

#### CSV

You can provide the requests as a CSV file, where the first row contains the headers and each subsequent row contains the values for a request.
The header row should look like this:

```csv
@timestamp,path,params,target_processing_time
```

The CSV-file may contain additional fields, but only the fields mentioned above will be used.

## License

Repeater is licensed under the Apache License, Version 2.0, ([LICENSE](LICENSE) or <https://www.apache.org/licenses/LICENSE-2.0>).

### <a name="license-contribution"></a>Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in repeater by you, as defined in the Apache-2.0 license, shall be licensed under the Apache-2.0 license, without any additional terms or conditions.
