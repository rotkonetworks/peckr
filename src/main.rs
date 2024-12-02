use std::time::Duration;
use std::net::IpAddr;
use clap::Parser;
use serde::Serialize;
use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence};
use tracing::{error, info};
use tokio::time::timeout;
use tokio::net::lookup_host;

#[derive(Parser)]
#[command(author, version, about = "ICMP ping utility with JSON output")]
struct Args {
    /// Target host to ping
    target: String,

    /// Stop after sending COUNT packets
    #[arg(short = 'c', long = "count", default_value_t = 30)]
    count: u32,

    /// Wait INTERVAL milliseconds between sending each packet
    #[arg(short = 'i', long = "interval", default_value_t = 100)]
    interval: u64,

    /// Time to wait for a response, in milliseconds
    #[arg(short = 'W', long = "timeout", default_value_t = 1000)]
    timeout: u64,

    /// Set Time to Live
    #[arg(short = 't', long = "ttl", default_value_t = 64)]
    ttl: u8,

    /// Maximum acceptable packet loss percentage
    #[arg(short = 'L', long = "max-loss", default_value_t = 5.0)]
    max_loss: f64,

    /// Maximum acceptable round-trip time in milliseconds
    #[arg(short = 'M', long = "max-latency", default_value_t = 800)]
    max_latency: u64,

    /// Server name for reporting (defaults to target)
    #[arg(short = 'n', long = "name")]
    server_name: Option<String>,

    /// Quiet output. Only show summary at end
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,
}

#[derive(Serialize)]
struct PingResult {
    checkname: String,
    servername: String,
    resulttype: String,
    success: bool,
    error: Option<String>,
    data: Option<PingData>,
}

#[derive(Serialize)]
struct PingData {
    latency: i64,
    packetloss: f64,
}

struct PingStats {
    sent: u32,
    received: u32,
    total_rtt: Duration,
}

impl PingStats {
    fn new() -> Self {
        Self {
            sent: 0,
            received: 0,
            total_rtt: Duration::ZERO,
        }
    }

    fn packet_loss(&self) -> f64 {
        if self.sent == 0 {
            return 100.0;
        }
        ((self.sent - self.received) as f64 / self.sent as f64) * 100.0
    }

    fn avg_rtt(&self) -> Duration {
        if self.received == 0 {
            return Duration::ZERO;
        }
        self.total_rtt / self.received
    }
}

async fn resolve_host(host: &str) -> Result<IpAddr, Box<dyn std::error::Error>> {
    // Try parsing as IP address first
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip);
    }

    // If not an IP, do DNS lookup
    let addrs = lookup_host(format!("{}:0", host)).await?;
    
    // Take the first IP address (similar to ping behavior)
    addrs
        .map(|socket_addr| socket_addr.ip())
        .next()
        .ok_or_else(|| "Could not resolve hostname".into())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let config = Args::parse();

    // Resolve the target hostname to an IP address
    let ip_addr = match resolve_host(&config.target).await {
        Ok(ip) => ip,
        Err(e) => {
            let result = PingResult {
                checkname: "ping".to_string(),
                servername: config.server_name.unwrap_or_else(|| config.target.clone()),
                resulttype: "site".to_string(),
                success: false,
                error: Some(format!("DNS resolution failed: {}", e)),
                data: None,
            };
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Err(e);
        }
    };

    if !config.quiet {
        info!("PING {} ({}) {} bytes of data",
            config.target,
            ip_addr,
            56 // Standard ICMP echo size
        );
    }

    let client = Client::new(&Config::builder()
        .kind(ICMP::V4)
        .ttl(config.ttl as u32)
        .build())?;

    let mut stats = PingStats::new();

    for sequence in 0..config.count {
        stats.sent += 1;

        let mut pinger = client.pinger(ip_addr, PingIdentifier(sequence as u16)).await;

        match timeout(
            Duration::from_millis(config.timeout),
            pinger.ping(PingSequence(sequence as u16), &[])
        ).await {
            Ok(Ok((_, rtt))) => {
                stats.received += 1;
                stats.total_rtt += rtt;
                if !config.quiet {
                    println!("64 bytes from {}: icmp_seq={} ttl={} time={:.2} ms",
                        ip_addr,
                        sequence,
                        config.ttl,
                        rtt.as_secs_f64() * 1000.0
                    );
                }
            }
            Ok(Err(e)) => {
                if !config.quiet {
                    error!("Ping failed for sequence {}: {}", sequence, e);
                }
            }
            Err(_) => {
                if !config.quiet {
                    error!("Request timeout for icmp_seq {}", sequence);
                }
            }
        }

        tokio::time::sleep(Duration::from_millis(config.interval)).await;
    }

    let packet_loss = stats.packet_loss();
    let avg_rtt = stats.avg_rtt().as_millis() as i64;

    let success = packet_loss <= config.max_loss
        && avg_rtt <= config.max_latency as i64
        && avg_rtt != 0;

    // Print ping-like statistics
    if !config.quiet {
        println!("\n--- {} ping statistics ---", config.target);
        println!("{} packets transmitted, {} received, {:.1}% packet loss, time {}ms",
            stats.sent,
            stats.received,
            packet_loss,
            stats.total_rtt.as_millis()
        );
        if stats.received > 0 {
            println!("rtt avg = {:.3} ms", avg_rtt as f64);
        }
    }

    if !success {
        error!(
            "Member: {} failed ping check - Loss: {:.2}% (Max: {}%) Latency: {}ms (Max: {}ms)",
            config.server_name.as_deref().unwrap_or(&config.target),
            packet_loss,
            config.max_loss,
            avg_rtt,
            config.max_latency
        );
    }

    let result = PingResult {
        checkname: "ping".to_string(),
        servername: config.server_name.unwrap_or_else(|| config.target.clone()),
        resulttype: "site".to_string(),
        success,
        error: None,
        data: Some(PingData {
            latency: avg_rtt,
            packetloss: packet_loss,
        }),
    };

    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}
