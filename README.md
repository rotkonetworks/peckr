# peckr

Packet Echo Check & Result reporter - an ICMP ping utility with JSON output.

## Usage

```
peckr [FLAGS] <target>
```

## Examples

Basic ping with defaults (30 packets, 100ms interval):
```bash
sudo peckr 1.1.1.1
```

Check with custom packet count and interval:
```bash
sudo peckr 1.1.1.1 -c 5 -i 500
```

Monitor with strict SLA requirements:
```bash
sudo peckr 1.1.1.1 -M 50 -L 1.0
```

Silent JSON output:
```bash
sudo peckr 1.1.1.1 -q
```

## Flags

- `-c, --count <COUNT>`: Stop after sending COUNT packets (default: 30)
- `-i, --interval <MS>`: Wait MS milliseconds between sending each packet (default: 100)
- `-W, --timeout <MS>`: Time to wait for response in milliseconds (default: 1000)
- `-t, --ttl <TTL>`: Set Time to Live (default: 64)
- `-L, --max-loss <LOSS>`: Maximum acceptable packet loss percentage (default: 5.0)
- `-M, --max-latency <MS>`: Maximum acceptable round-trip time in milliseconds (default: 800)
- `-n, --name <NAME>`: Server name for reporting (defaults to target)
- `-q, --quiet`: Quiet output. Only show JSON summary

## JSON Output

```json
{
  "checkname": "ping",
  "servername": "1.1.1.1",
  "resulttype": "site",
  "success": true,
  "error": null,
  "data": {
    "latency": 45,
    "packetloss": 0.0
  }
}
```

## Install

Download the latest release binary for your platform:
- Linux: `peckr-linux-x86_64`
- macOS: `peckr-macos-x86_64`

Make executable and move to path:
```bash
chmod +x peckr-linux-x86_64
sudo mv peckr-linux-x86_64 /usr/local/bin/peckr
```

## Build

```bash
cargo build --release
```

Requires root for raw socket access.
