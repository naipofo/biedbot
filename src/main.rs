mod api;
mod cache;
mod db;
mod secrets;

use crate::{api::BiedApi, secrets::Secrets};

use cache::BiedCache;
use db::BiedStore;
use secrets::get_secrets;
use std::sync::Arc;
use teloxide::{
    dispatching::{
        dialogue::{self, InMemStorage},
        UpdateFilterExt, UpdateHandler,
    },
    prelude::*,
    types::{ParseMode, Update},
    utils::command::BotCommands,
};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() {
    let Secrets {
        telegram_config,
        api_config,
    } = get_secrets();

    let bot = Bot::new(&telegram_config.bot_token).auto_send();
    let api = Arc::new(BiedApi::new(api_config));
    let store = Arc::new(Mutex::new(BiedStore::new("biedstore")));
    let cashe = Arc::new(Mutex::new(BiedCache::new()));

    let cfg = ConfigParameters {
        bot_admin: UserId(telegram_config.maintainer_id),
    };

    Dispatcher::builder(bot, schema())
        .dependencies(dptree::deps![
            InMemStorage::<State>::new(),
            api,
            store,
            cfg,
            cashe
        ])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

type MyDialogue = Dialogue<State, InMemStorage<State>>;
type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone)]
struct ConfigParameters {
    bot_admin: UserId,
}

#[derive(Clone, Default)]
pub enum State {
    #[default]
    Start,
    ReceiveSmsCode {
        title: String,
        phone_number: String,
    },
    ReceiveTitle {
        title: String,
        phone_number: String,
        sms_code: String,
    },
}

#[derive(BotCommands, Clone)]
#[command(rename = "lowercase", description = "These commands are supported:")]
enum Command {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "list all offers.")]
    Offers,
    #[command(description = "synchronize offers.")]
    Sync,
}

#[derive(BotCommands, Clone)]
#[command(rename = "lowercase", description = "Admin commands:")]
enum AdminCommand {
    #[command(description = "add an account.", parse_with = "split")]
    Add { title: String, phone_number: String },
    #[command(description = "cancel adding an account.")]
    Cancel,
    #[command(description = "list all added accounts.")]
    List,
    // TODO: Remove command
}

fn schema() -> UpdateHandler<Box<dyn std::error::Error + Send + Sync + 'static>> {
    use dptree::case;

    let command_handler = teloxide::filter_command::<Command, _>()
        .branch(case![Command::Help].endpoint(help))
        .branch(case![Command::Sync].endpoint(sync))
        .branch(case![Command::Offers].endpoint(offers));

    let admin_command_handler = teloxide::filter_command::<AdminCommand, _>()
        .filter(|msg: Message, cfg: ConfigParameters| {
            msg.from()
                .map(|user| user.id == cfg.bot_admin)
                .unwrap_or_default()
        })
        .branch(
            case![State::Start].branch(
                case![AdminCommand::Add {
                    title,
                    phone_number
                }]
                .endpoint(add_acconut),
            ),
        )
        .branch(case![AdminCommand::Cancel].endpoint(cancel))
        .branch(case![AdminCommand::List].endpoint(list));

    let message_handler = Update::filter_message()
        .branch(command_handler)
        .branch(admin_command_handler)
        .branch(
            case![State::ReceiveSmsCode {
                title,
                phone_number
            }]
            .endpoint(receive_sms_code),
        )
        .branch(
            case![State::ReceiveTitle {
                title,
                phone_number,
                sms_code
            }]
            .endpoint(recive_title),
        )
        .branch(dptree::endpoint(invalid_state));

    dialogue::enter::<Update, InMemStorage<State>, State, _>().branch(message_handler)
}

async fn help(bot: AutoSend<Bot>, msg: Message, cfg: ConfigParameters) -> HandlerResult {
    bot.send_message(
        msg.chat.id,
        format!(
            "{}\n\n{}",
            Command::descriptions().to_string(),
            if msg.from().unwrap().id == cfg.bot_admin {
                AdminCommand::descriptions().to_string()
            } else {
                "".to_string()
            }
        ),
    )
    .await?;
    Ok(())
}

async fn cancel(bot: AutoSend<Bot>, msg: Message, dialogue: MyDialogue) -> HandlerResult {
    bot.send_message(msg.chat.id, "Cancelling the dialogue.")
        .await?;
    dialogue.exit().await?;
    Ok(())
}

async fn list(bot: AutoSend<Bot>, msg: Message, store: Arc<Mutex<BiedStore>>) -> HandlerResult {
    bot.send_message(
        msg.chat.id,
        store
            .lock()
            .await
            .fetch_accounts()
            .into_iter()
            .map(|(title, user)| format!("*{title}* \\- {}", user))
            .collect::<Vec<_>>()
            .join("\n\n"),
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;

    Ok(())
}

async fn offers(bot: AutoSend<Bot>, msg: Message, cashe: Arc<Mutex<BiedCache>>) -> HandlerResult {
    // TODO: allow for accesing the phone number and 2d code
    // TODO: don't repeat same offers
    bot.send_message(
        msg.chat.id,
        format!(
            "Current offers:\n{}",
            cashe
                .lock()
                .await
                .offers
                .iter()
                .map(|e| e.short_display())
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .await?;
    Ok(())
}

async fn sync(
    bot: AutoSend<Bot>,
    msg: Message,
    cashe: Arc<Mutex<BiedCache>>,
    api: Arc<BiedApi>,
    store: Arc<Mutex<BiedStore>>,
) -> HandlerResult {
    let mut store = store.lock().await;
    let mut cashe = cashe.lock().await;
    match cashe.sync_offers(&mut store, &api).await {
        Ok(_) => {
            bot.send_message(msg.chat.id, "Synching finished.").await?;
        }
        Err(e) => {
            bot.send_message(msg.chat.id, format!("Synching failed: {:?}", e))
                .await?;
        }
    }
    Ok(())
}

async fn invalid_state(bot: AutoSend<Bot>, msg: Message) -> HandlerResult {
    bot.send_message(
        msg.chat.id,
        "Unable to handle the message. Type /help to see the usage.",
    )
    .await?;
    Ok(())
}

async fn add_acconut(
    bot: AutoSend<Bot>,
    msg: Message,
    api: Arc<BiedApi>,
    dialogue: MyDialogue,
    (title, phone_number): (String, String),
) -> HandlerResult {
    match api.send_sms_code(phone_number.clone()).await {
        Ok(_) => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "Creating accont {title}. Sending sms code to {phone_number}...\nWhat is it:",
                ),
            )
            .await?;
            dialogue
                .update(State::ReceiveSmsCode {
                    title,
                    phone_number,
                })
                .await?;
        }
        Err(e) => {
            bot.send_message(msg.chat.id, format!("Error sending sms message:{:?}", &e))
                .await?;
            dialogue.exit().await?;
        }
    }
    Ok(())
}

async fn receive_sms_code(
    bot: AutoSend<Bot>,
    msg: Message,
    api: Arc<BiedApi>,
    store: Arc<Mutex<BiedStore>>,
    dialogue: MyDialogue,
    (title, phone_number): (String, String),
) -> HandlerResult {
    match msg.text().map(ToOwned::to_owned) {
        Some(sms_code) => match api.calculate_next_step(phone_number.clone()).await {
            Ok(e) => match e {
                api::NextStep::NewAccount => {
                    bot.send_message(
                        msg.chat.id,
                        "This is a new account, what will be it's name:",
                    )
                    .await?;
                    dialogue
                        .update(State::ReceiveTitle {
                            sms_code,
                            title,
                            phone_number: phone_number,
                        })
                        .await?;
                }
                api::NextStep::AccountExist => {
                    let user = api.login(phone_number.clone(), sms_code).await.unwrap();
                    match store.lock().await.insert_account(&title, user) {
                        Ok(_) => {
                            bot.send_message(
                                msg.chat.id,
                                format!("Added user {title} with phone number {phone_number}!"),
                            )
                            .await?;
                            dialogue.exit().await?;
                        }
                        Err(e) => {
                            bot.send_message(
                                msg.chat.id,
                                format!("Error saving account: {:?}", &e),
                            )
                            .await?;
                        }
                    }
                }
            },
            Err(e) => {
                bot.send_message(msg.chat.id, format!("Error logging in:{:?}", &e))
                    .await?;
            }
        },
        None => {
            bot.send_message(msg.chat.id, "Please, send me the sms code.")
                .await?;
        }
    }
    Ok(())
}

async fn recive_title(
    bot: AutoSend<Bot>,
    msg: Message,
    dialogue: MyDialogue,
    api: Arc<BiedApi>,
    store: Arc<Mutex<BiedStore>>,
    (title, phone_number, sms_code): (String, String, String),
) -> HandlerResult {
    match msg.text().map(ToOwned::to_owned) {
        Some(name) => {
            match api
                .register(phone_number.clone(), sms_code.clone(), name.clone())
                .await
            {
                Ok(_) => {
                    bot.send_message(
                        dialogue.chat_id(),
                        format!("registered {name} succesfully."),
                    )
                    .await?;
                    let user = api.login(phone_number, sms_code).await.unwrap();
                    store.lock().await.insert_account(&title, user).unwrap();
                }
                Err(e) => panic!("{:?}", e),
            }

            dialogue.exit().await?;
        }
        None => {
            bot.send_message(msg.chat.id, "Please, send me your full name.")
                .await?;
        }
    }
    Ok(())
}
