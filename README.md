# linux-bsec-exporter

Export outputs of the
[*Bosch Sensortec Environmental Cluster (BSEC)*](https://www.bosch-sensortec.com/software-tools/software/bsec/)
as an HTTP endpoint scrapeable by Prometheus.


## Requirements

* A copy of the
  [BSEC library](https://www.bosch-sensortec.com/software-tools/software/bsec/)
  which needs to be obtained from Bosch.
  Be aware that it is proprietary software and you have to adhere to its
  license terms in addition to linux-bsec-exporter's license terms.
* A [BME-680 sensor](https://www.bosch-sensortec.com/products/environmental-sensors/gas-sensors/bme680/)
  connected via I2C to your system.
* A linux system.

I use linux-bsec-exporter with Raspian on a Raspberry Pi 3B+.


## Installation

1. Download the binary from GitHub or compile it yourself.
2. Place it on your device, e.g. in `/usr/local/bin/linux-bsec-exporter`.
3. Place a configuration file in `/etc/linux-bsec-exporter/config.tomal`.
   Look in `config.sample.toml` for a commented example.
4. Use the Ansible role provided in the roles directory to setup a service user and add a systemd service. (Or do this manually if you prefer.)


## Configuration

The configuration is read from `/etc/linux-bsec-exporter/config.toml` by
default.
A different path can be provided
with the `BSEC_CONFIG_PATH` environment variable.

See the `config.sample.toml` file for a documented example configuration.