use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_stream::StreamExt;
use tokio_util::codec::{Framed, LinesCodec};

use futures::SinkExt;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

/// Shorthand for the transmit half of the message channel.
type Tx = mpsc::UnboundedSender<String>;

/// Shorthand for the receive half of the message channel.
type Rx = mpsc::UnboundedReceiver<String>;

struct Shared {
    rooms: HashMap<String, HashMap<SocketAddr, Tx>>,
}

struct Peer {
    lines: Framed<TcpStream, LinesCodec>,
    rx: Rx,
    room: String, // Track which room this peer belongs to.
}

impl Shared {
    fn new() -> Self {
        Shared {
            rooms: HashMap::new(),
        }
    }

    async fn broadcast(&mut self, room: &str, sender: SocketAddr, message: &str) {
        if let Some(peers) = self.rooms.get_mut(room) {
            for (&addr, tx) in peers {
                if addr != sender {
                    let _ = tx.send(message.into());
                }
            }
        }
    }
}

impl Peer {
    async fn new(
        state: Arc<Mutex<Shared>>,
        lines: Framed<TcpStream, LinesCodec>,
        room_name: String,
    ) -> io::Result<Option<Peer>> {
        let addr = lines.get_ref().peer_addr()?;
        let (tx, rx) = mpsc::unbounded_channel();

        let mut state = state.lock().await;
        let peers = state
            .rooms
            .entry(room_name.clone())
            .or_insert_with(HashMap::new);
        peers.insert(addr, tx);

        // Prevent joining if the room already has 2 players
        if peers.len() >= 3 {
            println!("No room in lobby");
            return Ok(None);
        }
        Ok(Some(Peer {
            lines,
            rx,
            room: room_name,
        }))
    }
}

async fn process(
    state: Arc<Mutex<Shared>>,
    stream: TcpStream,
    addr: SocketAddr,
) -> Result<(), Box<dyn Error>> {
    let mut lines = Framed::new(stream, LinesCodec::new());

    lines.send("Please enter your room name:").await?;
    let room_name = match lines.next().await {
        Some(Ok(line)) => line,
        _ => {
            tracing::error!(
                "Failed to get room name from {}. Client disconnected.",
                addr
            );
            return Ok(());
        }
    };

    lines.send("Please enter your username:").await?;
    let username = match lines.next().await {
        Some(Ok(line)) => line,
        _ => {
            tracing::error!("Failed to get username from {}. Client disconnected.", addr);
            return Ok(());
        }
    };

    let peer = Peer::new(state.clone(), lines, room_name.clone()).await?;
    match peer {
        Some(mut peer) => {
            println!("Room found in lobby");
            {
                let mut state = state.lock().await;
                let msg = format!("{} has joined the room {}", username, room_name);
                tracing::info!("{}", msg);
                state.broadcast(&room_name, addr, &msg).await;
            }

            loop {
                tokio::select! {
                    Some(msg) = peer.rx.recv() => {
                        peer.lines.send(&msg).await?;
                    }
                    result = peer.lines.next() => match result {
                        Some(Ok(msg)) => {
                            let mut state = state.lock().await;
                            let msg = format!("{}: {}", username, msg);

                            state.broadcast(&peer.room, addr, &msg).await;
                        }
                        Some(Err(e)) => {
                            tracing::error!(
                                "an error occurred while processing messages for {}; error = {:?}",
                                username,
                                e
                            );
                        }
                        None => break,
                    },
                }
            }

            {
                let mut state = state.lock().await;
                if let Some(peers) = state.rooms.get_mut(&peer.room) {
                    peers.remove(&addr);
                }

                let msg = format!("{} has left the room {}", username, peer.room);
                tracing::info!("{}", msg);
                state.broadcast(&peer.room, addr, &msg).await;
            }
        }
        // TODO: Ensure that we return a message back when it's full
        None => {}
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    use tracing_subscriber::{fmt::format::FmtSpan, EnvFilter};

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("chat=info".parse()?))
        .with_span_events(FmtSpan::FULL)
        .init();

    let state = Arc::new(Mutex::new(Shared::new()));

    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());

    let listener = TcpListener::bind(&addr).await?;
    println!("server running on {}", addr);

    loop {
        let (stream, addr) = listener.accept().await?;

        let state = Arc::clone(&state);

        tokio::spawn(async move {
            if let Err(e) = process(state, stream, addr).await {
                eprintln!("an error occurred; error = {:?}", e);
            }
        });
    }
}
