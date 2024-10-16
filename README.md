# repeater

A command-line tool, `r7`, to parse and then repeat GET-requests of an access-log, against a different host with precise relative timing.

`r7` can also print all requests it parsed instead of repeating them. 

While repeating requests, `r7` will wait for the time difference between the current request and the previous request to maintain the relative timing. The time difference can be modified by applying a scaling factor to increase or decrease the load on the target system.

## Building

To build the project, you need to have Rust installed. You can install Rust by following the instructions at <https://www.rust-lang.org/tools/install>.

Once you have Rust installed, you can build the project by running the following command:

```sh
cargo build --all --release
```

The built binary will be available at `target/release/r7`.

## Usage

An `r7` command has the following structure:

```sh
$ r7 <command> [options and parameters]
```

For example to print the parsed requests from a file, you can run:

```sh
$ r7 print <path_of_file>
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

`r7` has been written under the pretense of having exported AWS Application Loadbalancer logs exported to S3 and then imported into an OpenSearch index. Therefore the file format is supposed to be an export from an ElasticSearch or OpenSearch index, either as CSV or JSONL.

The `@timestamp` field is expected to be in the ISO8601 format. The `params` and `path` fields are expected to be strings. These fields are expected in every line, so the file format should be consistent and can not contain other request methods where one of these three fields is missing (like for example `POST` which usually has no `params` field).

All requests in this file will be repeated as GET-requests against the specified host and scheme.

[!Warning]
Other request methods are not supported and may lead to `r7` terminating with an error (see paragraph about fields and their types) so be sure to clean the input file.

#### JSON

```json
{"_source":{"@timestamp":"","params":"","path":""}}
```

The JSON format may contain additional fields, but only the fields mentioned above will be used.

#### CSV

```csv
@timestamp,params,path
```

The CSV format may contain additional fields, but only the fields mentioned above will be used.

## License

Repeater is licensed under the Apache License, Version 2.0, ([LICENSE](LICENSE) or <https://www.apache.org/licenses/LICENSE-2.0>).

### <a name="license-contribution"></a>Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in repeater by you, as defined in the Apache-2.0 license, shall be licensed under the Apache-2.0 license, without any additional terms or conditions.
