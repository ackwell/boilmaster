# Mark XIV Thermocoil Boilmaster

Web service for Final Fantasy XIV game data and asset discovery.

## Installation

### Building From Source

**Requirements**

<!-- NOTE: See /rust-toolchain.toml when updating. -->
- [Rust](https://www.rust-lang.org/tools/install) >= 1.85.0

```bash
git clone https://github.com/ackwell/boilmaster
cd boilmaster
cargo run --release
```

It is recommended to edit your config in `boilmaster.toml` and at minimum change the admin username and password. See [the configuration section](#configuration) for more information.

### Docker Usage

boilmaster is published as a Docker image on the github container registery. An example `docker-compose.yml` such as the below can be used to bring the service online.

```yml
services:
  boilmaster:
    image: ghcr.io/ackwell/boilmaster:latest
    container_name: boilmaster
    environment:
      - BM_HTTP_ADMIN_AUTH_USERNAME="CHANGE-ME"
      - BM_HTTP_ADMIN_AUTH_PASSWORD="CHANGE-ME"
      # Other configuration here, see the Configuration section below for more information.
    volumes:
      # Need roughly 100gb of free space for patches
      - ${PWD}/persist:/app/persist
    ports:
      - 8080:8080
    restart: unless-stopped
```

## Configuration

The default configuration for boilmaster can be found in `boilmaster.toml`. This file can be considered a source of truth for all configuration options available.

In addition to the configuration file, all options may also be set via environment variables. The name of these variables is the same as their path in TOML; replacing `.` with `_`, in uppercase, with the prefix `BM_`. i.e. the config file key `http.api1.sheet.limit.default` can be set with the environment variable `BM_HTTP_API1_SHEET_LIMIT_DEFAULT`.

Configuration is only read during application startup, a restart is required if changes are made.

Before exposing the service to the public, it is strongly advised to change the `http.admin.auth.username` and `http.admin.auth.password` values.
