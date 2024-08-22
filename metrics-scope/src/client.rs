use std::net::ToSocketAddrs;
use std::time::Duration;
use std::{net::TcpStream, thread};

use metrics_exporter_scope::{protocol, ClientSettings};

use crate::{Event, EventSender};

fn read_remote(
    addr: &str,
    tx: &EventSender,
    sampling_interval: Duration,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr = addr.to_socket_addrs()?.next().ok_or("Invalid address")?;
    let mut client = TcpStream::connect_timeout(&addr, timeout)?;
    client.set_nodelay(true)?;
    client.set_read_timeout(Some(timeout))?;
    let version = protocol::read_version(&client).expect("Failed to read version");
    if version != protocol::VERSION {
        return Err(format!("Unsupported version: {}", version).into());
    }
    let settings = ClientSettings::new(sampling_interval);
    protocol::write_client_settings(&mut client, &settings)?;
    println!("Client connected: {}", addr);
    tx.send(Event::Connect).unwrap();
    loop {
        let packet = protocol::read_packet(&mut client)?;
        tx.send(Event::Packet(packet)).ok();
    }
}

pub fn reader(addr: &str, tx: EventSender, sampling_interval: Duration, timeout: Duration) {
    loop {
        if let Err(e) = read_remote(addr, &tx, sampling_interval, timeout) {
            tx.send(Event::Disconnect).ok();
            eprintln!("Error: {:?}", e);
        }
        thread::sleep(Duration::from_secs(1));
    }
}
