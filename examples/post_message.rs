//! Post a message to a Slack channel using `slack-morphism-reqwest`.
//!
//! Usage:
//! ```sh
//! SLACK_TEST_TOKEN=xoxb-... \
//! SLACK_TEST_CHANNEL=C12345 \
//!     cargo run --example post_message
//! ```

use rvstruct::ValueStruct;
use slack_morphism::api::SlackApiChatPostMessageRequest;
use slack_morphism::prelude::*;
use slack_morphism_reqwest::ReqwestConnector;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token_value = env::var("SLACK_TEST_TOKEN")
        .map_err(|_| "Set SLACK_TEST_TOKEN to a Slack bot or user token (xoxb-/xoxp-)")?;
    let channel = env::var("SLACK_TEST_CHANNEL")
        .map_err(|_| "Set SLACK_TEST_CHANNEL to the channel id to post in")?;

    let connector = ReqwestConnector::with_defaults()?;
    let client = SlackClient::new(connector);

    let token = SlackApiToken::new(token_value.into());
    let session = client.open_session(&token);

    let response = session
        .chat_post_message(&SlackApiChatPostMessageRequest::new(
            channel.into(),
            SlackMessageContent::new().with_text("hello from slack-morphism-reqwest".into()),
        ))
        .await?;

    println!(
        "posted message ts={} to channel {}",
        response.ts.value(),
        response.channel.value(),
    );

    Ok(())
}
