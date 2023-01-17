use std::{
    collections::HashMap,
    env,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use lazy_static::lazy_static;
use regex::Regex;
use serenity::{
    client::{Context, EventHandler},
    model::{channel::Message, gateway::Ready, prelude::UserId},
    prelude::{GatewayIntents, Mentionable, TypeMapKey},
    utils::MessageBuilder,
    Client,
};
use tokio::sync::RwLock;

#[derive(Clone)]
struct Record {
    pub last_mention: Option<Instant>,
    pub duration: Option<Duration>,
}

struct RecordTracker;

impl TypeMapKey for RecordTracker {
    type Value = Arc<RwLock<Record>>;
}

struct MentionCount;

impl TypeMapKey for MentionCount {
    type Value = Arc<RwLock<HashMap<UserId, AtomicUsize>>>;
}

struct LastReport;

impl TypeMapKey for LastReport {
    type Value = Arc<RwLock<Instant>>;
}

struct Handler;

#[serenity::async_trait]
impl EventHandler for Handler {
    async fn message(&self, context: Context, msg: Message) {
        lazy_static! {
            static ref RE: Regex = Regex::new(r#"\brust\b"#).unwrap();
        }

        if msg.author.id == context.cache.current_user().id
            || !(RE.is_match(&msg.content.to_ascii_lowercase()))
        {
            return;
        }

        let now = Instant::now();

        let mention_lock = {
            let data = context.data.read().await;
            data.get::<MentionCount>()
                .expect("Expected MentionCount in TypeMap.")
                .clone()
        };

        let mention_count = {
            let mut count = mention_lock.write().await;
            let count = count
                .entry(msg.author.id)
                .or_insert_with(|| AtomicUsize::new(0));
            count.fetch_add(1, Ordering::SeqCst);
            count.load(Ordering::SeqCst)
        };

        tracing::info!(
            "{} mentioned Rust {} times so far.",
            msg.author.name,
            mention_count
        );

        let should_report = {
            let data = context.data.read().await;
            let last_report = data
                .get::<LastReport>()
                .expect("Expected LastReport in TypeMap.")
                .clone();
            let mut last_report = last_report.write().await;
            let previous_report = *last_report;
            match now.checked_duration_since(previous_report) {
                Some(duration) if duration >= Duration::from_secs(60 * 60 * 24 * 5) => {
                    *last_report = now;
                    true
                }
                _ => false,
            }
        };

        if should_report {
            if let Some(guild_id) = msg.guild_id {
                if let Some(channels) = context.cache.guild_channels(guild_id) {
                    if let Some(channel) = channels.iter().find(|c| c.name == "random") {
                        let mut message_builder = MessageBuilder::new();
                        message_builder.push("👋 Hello everyone!\n\nIt's time to check who has mentioned Rust the most on the server. Here are the results:\n\n");

                        let data = mention_lock.read().await;
                        let mut mentions = data.iter().collect::<Vec<_>>();
                        mentions.sort_by_key(|(_, count)| count.load(Ordering::SeqCst));

                        mentions.iter().rev().take(10).for_each(|(user_id, count)| {
                            let count = count.load(Ordering::SeqCst);
                            message_builder
                                .push(count)
                                .push(" x ")
                                .push(user_id.mention())
                                .push("\n");
                        });

                        message_builder.push("\nCongratulations to the winners! 🎉");

                        if let Err(e) = channel
                            .send_message(&context.http, |m| {
                                m.embed(|e| {
                                    e.title("🦀 Rust Report 🦀")
                                        .description(message_builder.build())
                                        .color(0xdea584)
                                        .footer(|f| f.text("Made with  ❤️  and  🦀  by Near"))
                                })
                            })
                            .await
                        {
                            tracing::error!("An error occurred sending a report message: {}", e);
                        }
                    }
                }
            }
        }

        let record_lock = {
            let data = context.data.read().await;
            data.get::<RecordTracker>()
                .expect("Expected RecordTracker in TypeMap.")
                .clone()
        };
        let record = { record_lock.read().await.clone() };

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

        match (duration, record.duration) {
            (Some(current), Some(previous)) if current.gt(&previous) => {
                let seconds = current.as_secs();
                let minutes = seconds / 60;
                let hours = minutes / 60;
                let days = hours / 24;

                let formatted_time = if days > 0 {
                    format!("{} day(s) and {} hour(s)", days, hours % 24)
                } else if hours > 0 {
                    format!("{} hour(s) and {} minute(s)", hours, minutes % 60)
                } else if minutes > 0 {
                    format!("{} minute(s) and {} second(s)", minutes, seconds % 60)
                } else {
                    format!("{} seconds", seconds)
                };

                tracing::info!("New record: {}", formatted_time);

                if let Err(e) = msg
                    .channel_id
                    .send_message(&context, |m| {
                        m.embed(|e| {
                            e.title("🦀 Did somebody say Rust? 🦀")
                                .description(format!(
                                    "You lasted {} without mentioning Rust, that's a new record on this server!",
                                    formatted_time
                                ))
                                .color(0xdea584)
                                .footer(|f| f.text("Made with  ❤️  and  🦀  by Near"))
                        })
                    })
                    .await
                {
                    tracing::error!("An error occurred sending a new record message: {}", e);
                }

                {
                    let mut record = record_lock.write().await;
                    record.duration = if duration.is_none() {
                        Some(Duration::from_secs(0))
                    } else {
                        duration
                    }
                }
            }
            _ => {}
        }

        {
            let mut record = record_lock.write().await;
            record.last_mention = Some(now);
            if duration.is_none() {
                record.duration = Some(Duration::from_secs(0));
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
        data.insert::<RecordTracker>(Arc::new(RwLock::new(Record {
            last_mention: None,
            duration: None,
        })));
        data.insert::<MentionCount>(Arc::new(RwLock::new(HashMap::new())));
        data.insert::<LastReport>(Arc::new(RwLock::new(Instant::now())));
    }

    tracing::info!("Starting a new instance of the client.");

    if let Err(reason) = client.start().await {
        tracing::error!(
            "An unexpected client error occurred during runtime: {:?}",
            reason
        );
    }
}
