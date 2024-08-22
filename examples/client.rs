use std::{net::TcpStream, time::Duration};

use metrics_exporter_scope::{protocol, ClientSettings};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = TcpStream::connect("localhost:5001")?;
    let version = protocol::read_version(&client)?;
    if version != protocol::VERSION {
        return Err("Incompatible version".into());
    }
    let settings = ClientSettings::new(Duration::from_millis(100));
    protocol::write_client_settings(&mut client, &settings)?;
    loop {
        match protocol::read_packet(&mut client) {
            Ok(packet) => {
                dbg!(&packet);
            }
            Err(e) => {
                return Err(e.into());
            }
        }
    }
}
