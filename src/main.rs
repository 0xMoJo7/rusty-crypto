use dotenv;

use reqwest;

use std::collections::HashSet;
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use serenity::async_trait;
use serenity::prelude::*;
use serenity::prelude::Context;

use serenity::client::bridge::gateway::{ShardId, ShardManager};
use serenity::framework::standard::buckets::{LimitedFor, RevertBucket};
use serenity::framework::standard::macros::{check, command, group, help, hook};
use serenity::framework::standard::{
    help_commands,
    Args,
    CommandGroup,
    CommandOptions,
    CommandResult,
    DispatchError,
    HelpOptions,
    Reason,
    StandardFramework,
};
use serenity::http::Http;
use serenity::model::channel::{Channel, Message};
use serenity::model::gateway::{GatewayIntents, Ready};
use serenity::model::id::UserId;
use serenity::model::permissions::Permissions;
use serenity::prelude::*;
use serenity::utils::{content_safe, ContentSafeOptions};
use tokio::sync::Mutex;

struct ShardManagerContainer;

impl TypeMapKey for ShardManagerContainer {
    type Value = Arc<Mutex<ShardManager>>;
}

struct CommandCounter;

impl TypeMapKey for CommandCounter {
    type Value = HashMap<String, u64>;
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
    }
}

#[group]
#[commands(price)]
struct General;

#[help]
#[individual_command_tip = "Hello! Use `!` as a prefix for commands\n\n\
If you want more information about a specific command, just pass the command as argument."]
#[command_not_found_text = "Could not find: `{}`."]
#[max_levenshtein_distance(3)]
#[indention_prefix = "+"]
#[lacking_permissions = "Hide"]
#[lacking_role = "Nothing"]

async fn my_help(
    context: &Context,
    msg: &Message,
    args: Args,
    help_options: &'static HelpOptions,
    groups: &[&'static CommandGroup],
    owners: HashSet<UserId>,
) -> CommandResult {
    let _ = help_commands::with_embeds(context, msg, args, help_options, groups, owners).await;
    Ok(())
}

#[hook]
async fn before(ctx: &Context, msg: &Message, command_name: &str) -> bool {
    println!("Got command '{}' by user '{}'", command_name, msg.author.name);

    // Increment the number of times this command has been run once. If
    // the command's name does not exist in the counter, add a default
    // value of 0.
    let mut data = ctx.data.write().await;
    let counter = data.get_mut::<CommandCounter>().expect("Expected CommandCounter in TypeMap.");
    let entry = counter.entry(command_name.to_string()).or_insert(0);
    *entry += 1;

    true // if `before` returns false, command processing doesn't happen.
}

#[hook]
async fn after(_ctx: &Context, _msg: &Message, command_name: &str, command_result: CommandResult) {
    match command_result {
        Ok(()) => println!("Processed command '{}'", command_name),
        Err(why) => println!("Command '{}' returned error {:?}", command_name, why),
    }
}

#[hook]
async fn unknown_command(_ctx: &Context, _msg: &Message, unknown_command_name: &str) {
    println!("Could not find command named '{}'", unknown_command_name);
}

#[hook]
async fn normal_message(_ctx: &Context, msg: &Message) {
    println!("Message is not a command '{}'", msg.content);
}

#[hook]
async fn delay_action(ctx: &Context, msg: &Message) {
    // You may want to handle a Discord rate limit if this fails.
    let _ = msg.react(ctx, '⏱').await;
}

#[hook]
async fn dispatch_error(ctx: &Context, msg: &Message, error: DispatchError, _command_name: &str) {
    if let DispatchError::Ratelimited(info) = error {
        // We notify them only once.
        if info.is_first_try {
            let _ = msg
                .channel_id
                .say(&ctx.http, &format!("Try this again in {} seconds.", info.as_secs()))
                .await;
        }
    }
}

use serenity::futures::future::BoxFuture;
use serenity::FutureExt;
fn _dispatch_error_no_macro<'fut>(
    ctx: &'fut mut Context,
    msg: &'fut Message,
    error: DispatchError,
    _command_name: &str,
) -> BoxFuture<'fut, ()> {
    async move {
        if let DispatchError::Ratelimited(info) = error {
            if info.is_first_try {
                let _ = msg
                    .channel_id
                    .say(&ctx.http, &format!("Try this again in {} seconds.", info.as_secs()))
                    .await;
            }
        };
    }
    .boxed()
}

#[tokio::main]
async fn main() {
    let token = dotenv::var("DISCORD_TOKEN").unwrap();
    let http = Http::new(&token);
    
    let framework = StandardFramework::new()
        .configure(|c| c.prefix("!")
            .delimiters(vec![", ", " "])
            .with_whitespace(true))
                .before(before)
                .after(after)
                .unrecognised_command(unknown_command)
                .normal_message(normal_message)
                .bucket("emoji", |b| b.delay(5)).await
                .bucket("complicated", |b| b.limit(2).time_span(30).delay(5)
                    .limit_for(LimitedFor::Channel)
                    .await_ratelimits(1)
                    .delay_action(delay_action)).await
                .help(&MY_HELP)
                .group(&GENERAL_GROUP);

        let intents = GatewayIntents::all();
        let mut client = Client::builder(&token, intents)
            .event_handler(Handler)
            .framework(framework)
            .type_map_insert::<CommandCounter>(HashMap::default())
            .await
            .expect("Err creating client");
    
        {
            let mut data = client.data.write().await;
            data.insert::<ShardManagerContainer>(Arc::clone(&client.shard_manager));
        }
    
        if let Err(why) = client.start().await {
            println!("Client error: {:?}", why);
        }
}

#[command]
async fn price(ctx: &Context, msg: &Message) -> CommandResult {
    let etherscan_api_key = dotenv::var("ETHERSCAN_API_KEY").unwrap();
    let client = reqwest::Client::new();
    let response = client.get(format!("https://api.etherscan.io/api?module=stats&action=ethprice&apikey={}", etherscan_api_key))
        .send()
        .await
        .unwrap();
    match response.status() {
        reqwest::StatusCode::OK => {
            let body = response.text().await.unwrap();
            let json: Value = serde_json::from_str(&body).unwrap();
            let price = json["result"]["ethusd"].as_str().unwrap();
            msg.reply(&ctx.http, format!("The current price of ETH is ${}", price)).await?;
        },
        _ => {
            msg.reply(&ctx.http, "Something went wrong").await?;
        }
    }
    Ok(())
}
