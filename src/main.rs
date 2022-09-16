use std::{
    env,
    sync::Arc,
    time::{Duration, Instant},
};

use lazy_static::lazy_static;
use regex::Regex;
use serenity::{
    client::{Context, EventHandler},
    model::{channel::Message, gateway::Ready},
    prelude::{GatewayIntents, TypeMapKey},
    Client,
};
use tokio::sync::RwLock;

#[derive(Copy, Clone)]
struct Record {
    pub last_mention: Option<Instant>,
    pub duration: Option<Duration>,
}

struct SharedData;

impl TypeMapKey for SharedData {
    type Value = Arc<RwLock<Record>>;
}

struct Handler;

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn message(&self, context: Context, msg: Message) {
        lazy_static! {
            static ref RE: Regex = Regex::new(r#"\brust\b"#).unwrap();
        }

        if msg.author.bot {
            return;
        }

        let now = Instant::now();

        if RE.is_match(&msg.content.to_ascii_lowercase()) {
            tracing::info!("Somebody mentioned Rust.");

            let record_lock = {
                let data_read = context.data.read().await;
                data_read
                    .get::<SharedData>()
                    .expect("Expected SharedData in TypeMap.")
                    .clone()
            };
            let record = { *record_lock.read().await };

            let duration = if let Some(last_mention) = record.last_mention {
                now.checked_duration_since(last_mention)
            } else {
                None
            };

            tracing::info!(
                "Previous record duration was {:?}, the current duration was {:?}",
                record.duration,
                duration
            );

            if let (Some(current), Some(previous)) = (duration, record.duration) {
                if current.gt(&previous) {
                    let seconds = current.as_secs();
                    let minutes = seconds / 60;
                    let hours = minutes / 60;
                    let days = hours / 24;

                    let humantime = if days > 0 {
                        format!("{} days and {} hours", days, hours % 24)
                    } else if hours > 0 {
                        format!("{} hours and {} minutes", hours, minutes % 60)
                    } else if minutes > 0 {
                        format!("{} minutes", minutes)
                    } else {
                        format!("{} seconds", seconds)
                    };

                    tracing::info!("New record: {}", humantime);

                    // if let Err(e) = msg
                    //     .channel_id
                    //     .send_message(&context, |m| {
                    //         m.content(format!(
                    //             "You lasted {} without mentioning Rust, that's a new record!",
                    //             humantime
                    //         ))
                    //     })
                    //     .await
                    // {
                    //     tracing::error!("An error occurred sending a new record message: {}", e);
                    // }

                    {
                        let mut writable_record = record_lock.write().await;
                        writable_record.duration = if duration == None {
                            Some(Duration::from_secs(0))
                        } else {
                            duration
                        };
                    }
                }
            }

            {
                let mut writable_record = record_lock.write().await;
                writable_record.last_mention = Some(now);
                if duration == None {
                    writable_record.duration = Some(Duration::from_secs(0));
                };
            }
        }
    }

    async fn ready(&self, _: Context, data: Ready) {
        tracing::info!("{} is connected and running.", data.user.name);
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let token =
        env::var("DISCORD_TOKEN").expect("Could not find the DISCORD_TOKEN environment variable.");
    let intents =
        GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT | GatewayIntents::GUILDS;
    let mut client = Client::builder(&token, intents)
        .event_handler(Handler)
        .await
        .expect("There was an unexpected error while attempting to create a client.");

    {
        let mut data = client.data.write().await;
        data.insert::<SharedData>(Arc::new(RwLock::new(Record {
            last_mention: None,
            duration: None,
        })))
    }

    tracing::info!("Starting a new instance of the client.");

    if let Err(reason) = client.start().await {
        tracing::error!(
            "An unexpected client error occurred during runtime: {:?}",
            reason
        );
    }
}
