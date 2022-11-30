mod api;
mod cache;
mod db;
mod secrets;

use crate::{api::BiedApi, secrets::Secrets};

use api::{AuthData, Offer};
use cache::BiedCache;
use db::BiedStore;
use secrets::get_secrets;
use std::sync::Arc;
use teloxide::{
    dispatching::{UpdateFilterExt, UpdateHandler},
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, InputFile, ParseMode, Update},
    utils::command::BotCommands,
};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() {
    let Secrets {
        telegram_config,
        api_config,
        ean_frontend,
        cdn_root,
    } = get_secrets();

    let bot = Bot::new(&telegram_config.bot_token);
    let api = Arc::new(BiedApi::new(api_config));
    let store = Arc::new(Mutex::new(BiedStore::new("biedstore")));
    let cashe = Arc::new(Mutex::new(BiedCache::new()));

    let cfg = ConfigParameters {
        bot_admins: telegram_config
            .maintainer_ids
            .iter()
            .map(|e| UserId(*e))
            .collect(),
        ean_frontend,
        cdn_root,
    };

    Dispatcher::builder(bot, schema())
        .dependencies(dptree::deps![api, store, cfg, cashe])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone)]
struct ConfigParameters {
    bot_admins: Vec<UserId>,
    ean_frontend: String,
    cdn_root: String,
}

impl ConfigParameters {
    fn is_admin(&self, id: &UserId) -> bool {
        self.bot_admins.contains(&id)
    }
}

#[derive(BotCommands, Clone)]
#[command(
    rename_rule = "lowercase",
    description = "These commands are supported:"
)]
enum Command {
    #[command(description = "display this text.")]
    Help,
    #[command(description = "list all offers.")]
    Offers,
    #[command(description = "synchronize offers.")]
    Sync,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Admin commands:")]
enum AdminCommand {
    #[command(
        description = "add an account. Usage: /add title ean phone user1 user2 csrf",
        parse_with = "split"
    )]
    Add {
        title: String,
        card_number: String,
        phone_number: String,
        users1: String,
        users2: String,
        csrf_token: String,
    },
    #[command(description = "cancel adding an account.")]
    Cancel,
    #[command(description = "list all added accounts.")]
    List,
    #[command(description = "rename an account.", parse_with = "split")]
    Rename { old: String, new: String },
    #[command(description = "remove account with the specified title.")]
    Remove { title: String },
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
                .map(|user| cfg.is_admin(&user.id))
                .unwrap_or_default()
        })
        .branch(
            case![AdminCommand::Add {
                title,
                card_number,
                phone_number,
                users1,
                users2,
                csrf_token
            }]
            .endpoint(add_acconut),
        )
        .branch(case![AdminCommand::List].endpoint(list))
        .branch(case![AdminCommand::Rename { old, new }].endpoint(rename))
        .branch(case![AdminCommand::Remove { title }].endpoint(remove));

    let message_handler = Update::filter_message()
        .branch(command_handler)
        .branch(admin_command_handler)
        .branch(dptree::endpoint(invalid_state));

    dptree::entry()
        .branch(Update::filter_message().branch(message_handler))
        .branch(Update::filter_callback_query().endpoint(endpoint_button))
}

async fn help(bot: Bot, msg: Message, cfg: ConfigParameters) -> HandlerResult {
    bot.send_message(
        msg.chat.id,
        format!(
            "{}\n\n{}",
            Command::descriptions().to_string(),
            if cfg.is_admin(&msg.from().unwrap().id) {
                AdminCommand::descriptions().to_string()
            } else {
                "".to_string()
            }
        ),
    )
    .await?;
    Ok(())
}

async fn list(bot: Bot, msg: Message, store: Arc<Mutex<BiedStore>>) -> HandlerResult {
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

async fn rename(
    bot: Bot,
    msg: Message,
    store: Arc<Mutex<BiedStore>>,
    (old, new): (String, String),
) -> HandlerResult {
    bot.send_message(
        msg.chat.id,
        match store.lock().await.rename_account(&old, &new) {
            Ok(_) => format!("Renamed user {} to {}", old, new),
            Err(e) => format!("Error renaming user: {:?}", e),
        },
    )
    .await?;
    Ok(())
}

async fn remove(
    bot: Bot,
    msg: Message,
    store: Arc<Mutex<BiedStore>>,
    title: String,
) -> HandlerResult {
    bot.send_message(
        msg.chat.id,
        match store.lock().await.remove_account(&title) {
            Ok(u) => format!("Removed user {}", u),
            Err(e) => format!("Error removing user: {:?}", e),
        },
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;
    Ok(())
}

async fn offers(bot: Bot, msg: Message, cashe: Arc<Mutex<BiedCache>>) -> HandlerResult {
    // TODO: don't repeat same offers
    let offers = &cashe.lock().await.offers;
    bot.send_message(
        msg.chat.id,
        format!(
            "Current offers:\n\n{}",
            offers
                .iter()
                .map(|e| format!(
                    "{}:\n{}\n",
                    e.0,
                    e.1.iter()
                        .map(|e| e.short_display())
                        .collect::<Vec<_>>()
                        .join("\n")
                ))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .reply_markup(make_accounts_keyboard(
        // TODO: don't clone here
        offers.into_iter().map(|e| e.0.clone()).collect(),
    ))
    .await?;
    Ok(())
}

fn make_accounts_keyboard(names: Vec<String>) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(
        names
            .chunks(2)
            .map(|e| {
                e.iter()
                    .map(|n| InlineKeyboardButton::callback(n.clone(), n.clone()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>(),
    )
}

async fn endpoint_button(
    bot: Bot,
    q: CallbackQuery,
    store: Arc<Mutex<BiedStore>>,
    cashe: Arc<Mutex<BiedCache>>,
    cfg: ConfigParameters,
) -> HandlerResult {
    bot.answer_callback_query(q.id).await?;

    let title = q.data.unwrap().clone();
    let card_number = store
        .lock()
        .await
        .fetch_account(&title)
        .unwrap()
        .card_number;
    let mut cashe = cashe.lock().await;
    let offers = cashe.get_offers(&title).await.unwrap();

    for o in offers {
        let Offer {
            name,
            details,
            limit,
            image,
            regular_price,
            regular_price_unit,
            offer_price,
            offer_price_unit,
            ..
        } = o;
        let text = format!("<b>{name}</b>\n<code>{details}</code>\n{limit}\n{regular_price} -> {offer_price}\n{regular_price_unit} -> {offer_price_unit}");
        match image {
            Some(img) => {
                let pic = reqwest::get(format!("{}{}", cfg.cdn_root, img))
                    .await?
                    .bytes()
                    .await?;
                bot.send_photo(q.from.id, InputFile::memory(pic))
                    .caption(text)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
            None => {
                bot.send_message(q.from.id, text)
                    .parse_mode(ParseMode::Html)
                    .await?;
            }
        }
    }
    bot.send_message(
        q.from.id,
        format!("[View card\u{1F4B3}]({}{})", cfg.ean_frontend, card_number),
    )
    .parse_mode(ParseMode::MarkdownV2)
    .await?;
    Ok(())
}

async fn sync(
    bot: Bot,
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

async fn invalid_state(bot: Bot, msg: Message) -> HandlerResult {
    bot.send_message(
        msg.chat.id,
        "Unable to handle the message. Type /help to see the usage.",
    )
    .await?;
    Ok(())
}

async fn add_acconut(
    bot: Bot,
    msg: Message,
    store: Arc<Mutex<BiedStore>>,
    (title, card_number, phone_number, users1, users2, csrf_token): (
        String,
        String,
        String,
        String,
        String,
        String,
    ),
) -> HandlerResult {
    let mut store = store.lock().await;
    bot.send_message(
        msg.chat.id,
        match store.insert_account(
            &title,
            api::AuthenticatedUser {
                phone_number,
                card_number,
                auth: AuthData {
                    users1,
                    users2,
                    csrf_token,
                },
            },
        ) {
            Ok(_) => "Account added succesfully".to_string(),
            Err(e) => format!("Error adding account: {:?}", e),
        },
    )
    .await?;
    Ok(())
}
