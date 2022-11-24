use std::collections::BTreeMap;
use std::time::Instant;

use tts::*;
use twitch_irc::login::StaticLoginCredentials;
use twitch_irc::message::ServerMessage;
use twitch_irc::ClientConfig;
use twitch_irc::SecureTCPTransport;
use twitch_irc::TwitchIRCClient;

use tokio::signal;
use tokio::sync::{broadcast, mpsc};

// settings
// streamer to get chat messages from
const STREAMER: &str = "dougdougw";
// how many users have to say the same phrase before tts speaks it
const USER_THRESHOLD: usize = 10;
// how many seconds before a message gets deleted
const MESSAGE_LIFETIME: u64 = 10;

//TODO! consider moving struct & impl to a module
struct MessageData {
    users: Vec<String>,
    modified: Instant,
}

impl MessageData {
    pub fn new(user: String) -> MessageData {
        MessageData {
            users: vec![user],
            modified: Instant::now(),
        }
    }
}

// forks that change the behavior will likely edit this fn
//TODO! use better error handling
async fn manage_messages(mut incoming_messages: mspc::UnboundedReceiver<ServerMessage>) -> Result<(), tts::Error> {
    // BTreeMap of recent messages, sorted by message content
    // BTreeMap<Message, Vec<UserID>>
    let mut messages: BTreeMap<String, MessageData> = BTreeMap::new();
    let mut tts = Tts::default()?;

    while let Some(message) = incoming_messages.recv().await {
        match message {
            ServerMessage::Privmsg(msg) => {
                // add message to map
                //TODO! consider fuzzy finding instead of lowercasing
                let text_normalized = msg.message_text.as_str().to_lowercase();

                // if message is not in map, add it
                if !messages.contains_key(&text_normalized) {
                    messages.insert(text_normalized, MessageData::new(msg.sender.id));
                } else {
                    // if message is in map, check if this user is in map
                    // this unwrap cannot fail because we just checked that it contains this key
                    let users = &mut messages.get_mut(&text_normalized).unwrap().users;
                    // if not, add them
                    if !users.contains(&msg.sender.id) {
                        users.push(msg.sender.id);
                    }
                }
            }
            _ => {}
        }

        // handle messages
        //TODO! is there a better way to do this?
        let mut deleted_messages: Vec<String> = Vec::new();
        for (message, data) in &messages {
            // if message is popular, speak it with tts
            if data.users.len() >= USER_THRESHOLD {
                println!("Speaking \"{}\"", message);
                deleted_messages.push(message.clone());
                tts.speak(message, false)?;
            }

            // if message is old, remove it
            if data.modified.elapsed().as_secs() >= MESSAGE_LIFETIME {
                deleted_messages.push(message.clone());
            }
        }

        for message in deleted_messages {
            messages.remove(&message);
        }
    }
    //TODO! should we return an error if there are no more messages?
    Ok(())
}

//TODO! use better error handling
#[tokio::main]
pub async fn main() -> Result<(), tts::Error> {
    // create shutdown broadcast channel and shutdown finish channel
    //TODO! wrap in a struct
    let (shutdown_send, mut shutdown_recv) = broadcast::channel(2);
    let (drop_to_shutdown, mut error_to_shutdown) = mspc::channel(1);

    // default configuration is to join chat as anonymous.
    let config = ClientConfig::default();
    let (mut incoming_messages, client) =
        TwitchIRCClient::<SecureTCPTransport, StaticLoginCredentials>::new(config);

    // start consuming incoming messages,
    // otherwise they will back up.
    tokio::select! {
        // if an error is recieved, return the error
        // we could just move `drop_to_shutdown` into the fn, but i prefer
        // each thread to have a Sender
        res = manage_messages(incoming_messages, drop_to_shutdown.clone()) => res?,
        _ = shutdown_recv.recv() => {
            // if a shutdown signal is recieved, return
            return Ok(());
        }
    };
    
    //TODO! consider making client management and the ctrl+c handler seperate
    
    // join a twitch channel
    // This function only returns an error if the passed channel login name is malformed,
    // so in this simple case where the channel name is hardcoded we can ignore the potential
    // error with `unwrap`.
    client.join(STREAMER.to_owned()).unwrap();

    // wait for ctrl+c
    signal::ctrl_c().await.expect("failed to wait for ctrl+c");
    // after ctrl+c, start shutdown procedure
    
    // leave the twitch channel
    client.part(STREAMER.to_owned());
    
    // tell other threads to shutdown
    shutdown_send.send(1).unwrap();
    
    // let other threads know we are ready to shutdown
    drop(drop_to_shutdown);
    
    // When every sender has gone out of scope, the recv call
    // will return with an error. We ignore the error.
    let _ = recv.recv().await;

    Ok(())
}
