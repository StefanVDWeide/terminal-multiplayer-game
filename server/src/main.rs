use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_stream::StreamExt;
use tokio_util::codec::{Framed, LinesCodec};

use futures::SinkExt;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use uuid::Uuid;

/// Shorthand for the transmit half of the message channel.
type Tx = mpsc::UnboundedSender<String>;

/// Shorthand for the receive half of the message channel.
type Rx = mpsc::UnboundedReceiver<String>;

struct Shared {
    rooms: HashMap<String, RoomState>,
}

struct Player {
    name: String,
    sender: Tx,
    hp: i32,
    defense: i32,
}

struct RoomState {
    peers: HashMap<Uuid, Player>,
    turn: Option<Uuid>, // Track whose turn it is
}

struct Peer {
    lines: Framed<TcpStream, LinesCodec>,
    rx: Rx,
    room: String,
}

impl Shared {
    fn new() -> Self {
        Shared {
            rooms: HashMap::new(),
        }
    }

    async fn broadcast(&self, room: &str, user_id: Uuid, message: &str) {
        if let Some(room_state) = self.rooms.get(room) {
            for (&id, player) in &room_state.peers {
                if id != user_id {
                    let _ = player.sender.send(message.into());
                }
            }
        }
    }

    async fn next_turn(&mut self, room: &str) {
        if let Some(room_state) = self.rooms.get_mut(room) {
            let peers: Vec<Uuid> = room_state.peers.keys().copied().collect();
            room_state.turn = match &room_state.turn {
                Some(current) => {
                    let index = peers.iter().position(|&addr| addr == *current).unwrap();
                    let next_index = (index + 1) % peers.len();
                    Some(peers[next_index])
                }
                None => peers.get(0).copied(),
            };
        }
    }

    // TODO: This is not used correctly, the player ID is the one attacking so the other should be used for applying damage
    async fn apply_attack(&mut self, room: &str, id: Uuid, damage: i32) -> Option<String> {
        println!("{}", id);
        if let Some(room_state) = self.rooms.get_mut(room) {
            if let Some(player) = room_state.peers.get_mut(&id) {
                let total_damage = player.defense - damage;
                player.hp -= total_damage;
                if player.hp <= 0 {
                    return Some(format!("{} has lost!", player.name));
                }
            }
        }
        None
    }
}

impl Peer {
    async fn new(
        state: Arc<Mutex<Shared>>,
        mut lines: Framed<TcpStream, LinesCodec>,
        room_name: String,
        user_name: String,
        id: Uuid,
    ) -> io::Result<Option<Peer>> {
        let (tx, rx) = mpsc::unbounded_channel();

        let mut state = state.lock().await;
        let room_state = state
            .rooms
            .entry(room_name.clone())
            .or_insert_with(|| RoomState {
                peers: HashMap::new(),
                turn: None,
            });

        // Check if there are 2 or more people in the room already and prevent the next person from joining
        if room_state.peers.len() >= 2 {
            let _ = lines.send("No room in lobby").await;
            println!("No room in lobby");
            return Ok(None);
        }

        let player_struct = Player {
            name: user_name,
            sender: tx,
            hp: 10,
            defense: 10,
        };

        // Create the player data -> Refactor this. Probably use a UUID instead of the addr
        room_state.peers.insert(id, player_struct);
        if room_state.peers.len() == 2 {
            room_state.turn = Some(id); // First player to join attacks first
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

    // Ask the user for the room name that they want to join
    lines.send("Please enter your room name:").await?;
    let room_name = match lines.next().await {
        Some(Ok(line)) => line,
        _ => {
            eprintln!(
                "Failed to get room name from {}. Client disconnected.",
                addr
            );
            return Ok(());
        }
    };

    // Ask the user for their username
    lines.send("Please enter your username:").await?;
    let user_name = match lines.next().await {
        Some(Ok(user_name)) => user_name,
        _ => {
            eprintln!("Failed to get username from {}. Client disconnected.", addr);
            return Ok(());
        }
    };

    // Generate an unique ID for each player
    let id = Uuid::new_v4();

    println!("{}", id);

    let peer = Peer::new(state.clone(), lines, room_name.clone(), user_name, id).await?;
    if let Some(mut peer) = peer {
        let mut state_lock = state.lock().await;
        let room_state = state_lock.rooms.get_mut(&room_name).unwrap();
        if let Some(turn) = room_state.turn {
            if turn == id {
                let msg = "Your turn";
                peer.lines.send(msg).await?;
            }
        }
        drop(state_lock); // Release lock immediately after use

        loop {
            tokio::select! {
                Some(msg) = peer.rx.recv() => {
                    peer.lines.send(&msg).await?;
                }
                result = peer.lines.next() => match result {
                    Some(Ok(msg)) => {
                        let mut state = state.lock().await;
                        println!("{}", &msg);
                        if msg.contains("attack") {
                            let value: Value = serde_json::from_str(&msg).unwrap();
                            println!("{:?}", value);
                            if let Some(attack_value) = value.get("attack").and_then(|v| v.as_i64()).map(|v| v as i32) {
                                // TODO: Make this more clear for the players. What should be sent back when an attack has been done?
                                if let Some(result_message) = state.apply_attack(&peer.room, id, attack_value).await {
                                    state.broadcast(&peer.room, id, &result_message).await;
                                    break;
                                }
                                state.next_turn(&peer.room).await;
                                state.broadcast(&peer.room, id, "next turn").await;
                            }
                        }
                    }
                    Some(Err(e)) => {
                        eprintln!("an error occurred while processing messages for {}; error = {:?}", addr, e);
                    }
                    None => break,
                },
            }
        }
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
