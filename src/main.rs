use std::time::Duration;
use std::net::IpAddr;
use std::sync::Arc;
use clap::Parser;
use serde::Serialize;
use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence};
use tracing::{error, info};
use tokio::time::timeout;
use tokio::net::lookup_host;
use tokio::sync::Mutex;
use tokio::signal;

#[derive(Parser, Clone)]
#[command(author, version, about = "ICMP ping utility with JSON output")]
struct Args {
    /// Target host to ping
    target: String,

    /// Stop after sending COUNT packets (0 for endless mode)
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

#[derive(Serialize, Clone)]
struct PingResult {
    checkname: String,
    servername: String,
    resulttype: String,
    success: bool,
    error: Option<String>,
    data: Option<PingData>,
}

#[derive(Serialize, Clone)]
struct PingData {
    latency: i64,
    packetloss: f64,
    packets_sent: u32,
    packets_received: u32,
}

#[derive(Clone)]
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

    fn update_with_success(&mut self, rtt: Duration) {
        self.sent += 1;
        self.received += 1;
        self.total_rtt += rtt;
    }

    fn update_with_failure(&mut self) {
        self.sent += 1;
    }
}

async fn resolve_host(host: &str) -> Result<IpAddr, Box<dyn std::error::Error>> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip);
    }

    let addrs = lookup_host(format!("{}:0", host)).await?;
    addrs
        .map(|socket_addr| socket_addr.ip())
        .next()
        .ok_or_else(|| "Could not resolve hostname".into())
}

async fn send_single_ping(
    client: &Client,
    ip_addr: IpAddr,
    sequence: u32,
    timeout_duration: Duration,
) -> Result<Duration, Box<dyn std::error::Error>> {
    let mut pinger = client.pinger(ip_addr, PingIdentifier(sequence as u16)).await;
    
    let result = timeout(
        timeout_duration,
        pinger.ping(PingSequence(sequence as u16), &[])
    ).await;

    match result {
        Ok(Ok((_, rtt))) => Ok(rtt),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Err("Request timeout".into()),
    }
}

async fn print_ping_result(ip_addr: IpAddr, sequence: u32, ttl: u8, rtt: Duration) {
    println!(
        "64 bytes from {}: icmp_seq={} ttl={} time={:.2} ms",
        ip_addr,
        sequence,
        ttl,
        rtt.as_secs_f64() * 1000.0
    );
}

async fn print_statistics(target: &str, stats: &PingStats) {
    println!("\n--- {} ping statistics ---", target);
    println!(
        "{} packets transmitted, {} received, {:.1}% packet loss, time {}ms",
        stats.sent,
        stats.received,
        stats.packet_loss(),
        stats.total_rtt.as_millis()
    );
    
    if stats.received > 0 {
        println!("rtt avg = {:.3} ms", stats.avg_rtt().as_millis());
    }
}

fn create_result(config: &Args, stats: &PingStats) -> PingResult {
    let packet_loss = stats.packet_loss();
    let avg_rtt = stats.avg_rtt().as_millis() as i64;
    
    let success = packet_loss <= config.max_loss
        && avg_rtt <= config.max_latency as i64
        && avg_rtt != 0;

    PingResult {
        checkname: "ping".to_string(),
        servername: config.server_name.clone().unwrap_or_else(|| config.target.clone()),
        resulttype: "site".to_string(),
        success,
        error: None,
        data: Some(PingData {
            latency: avg_rtt,
            packetloss: packet_loss,
            packets_sent: stats.sent,
            packets_received: stats.received,
        }),
    }
}

async fn monitor_ctrl_c() -> Result<(), tokio::io::Error> {
    signal::ctrl_c().await
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let config = Args::parse();

    let ip_addr = match resolve_host(&config.target).await {
        Ok(ip) => ip,
        Err(e) => {
            let result = PingResult {
                checkname: "ping".to_string(),
                servername: config.server_name.clone().unwrap_or_else(|| config.target.clone()),
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
        info!("PING {} ({}) {} bytes of data", config.target, ip_addr, 56);
        if config.count == 0 {
            info!("Running in endless mode. Press Ctrl+C to stop.");
        }
    }

    let client = Client::new(&Config::builder()
        .kind(ICMP::V4)
        .ttl(config.ttl as u32)
        .build())?;

    let stats = Arc::new(Mutex::new(PingStats::new()));
    let mut sequence = 0;
    
    let ctrl_c = tokio::spawn(monitor_ctrl_c());
    
    loop {
        if config.count > 0 && sequence >= config.count {
            break;
        }

        let ping_result = send_single_ping(
            &client,
            ip_addr,
            sequence,
            Duration::from_millis(config.timeout),
        ).await;

        let mut stats_guard = stats.lock().await;
        match ping_result {
            Ok(rtt) => {
                stats_guard.update_with_success(rtt);
                if !config.quiet {
                    drop(stats_guard);
                    print_ping_result(ip_addr, sequence, config.ttl, rtt).await;
                }
            }
            Err(e) => {
                stats_guard.update_with_failure();
                if !config.quiet {
                    error!("Ping failed for sequence {}: {}", sequence, e);
                }
            }
        }

        sequence += 1;
        
        if ctrl_c.is_finished() {
            break;
        }
        
        tokio::time::sleep(Duration::from_millis(config.interval)).await;
    }

    if !config.quiet {
        let stats_guard = stats.lock().await;
        print_statistics(&config.target, &stats_guard).await;
    }

    let final_stats = stats.lock().await;
    let result = create_result(&config, &final_stats);
    println!("{}", serde_json::to_string_pretty(&result)?);

    Ok(())
}
