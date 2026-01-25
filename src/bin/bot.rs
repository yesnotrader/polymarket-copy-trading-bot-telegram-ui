//! Telegram Bot UI for Polymarket Copy Trading Bot
//! 
//! Provides a Telegram interface to:
//! - Manage environment variables
//! - Validate setup
//! - Run bot binaries (auth, stream, engine, test, watch)
//! - View logs in real-time
//! - Stop running processes

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Command as TokioCommand, Child as TokioChild};
use tokio::sync::Mutex;
use std::process::Stdio;

// Process management - store tokio Child processes
type ProcessMap = Arc<Mutex<HashMap<String, (TokioChild, MessageId)>>>;

// Helper functions for user config management
fn get_user_config_path(user_id: i64) -> PathBuf {
    PathBuf::from("users").join(format!("{}", user_id))
}

fn ensure_users_directory() -> std::io::Result<()> {
    fs::create_dir_all("users")
}

fn initialize_user_config(user_id: i64) -> std::io::Result<()> {
    ensure_users_directory()?;
    let user_config_path = get_user_config_path(user_id);
    
    // If user config already exists, don't overwrite
    if user_config_path.exists() {
        return Ok(());
    }
    
    // Try to copy from .config.example, otherwise create a template
    let template_content = if Path::new(".config.example").exists() {
        fs::read_to_string(".config.example")?
    } else {
        // Create a basic template if .config.example doesn't exist
        "# User Configuration File\n# Generated automatically\n\n".to_string()
    };
    
    fs::write(&user_config_path, template_content)?;
    Ok(())
}

fn read_user_config(user_id: i64) -> String {
    let user_config_path = get_user_config_path(user_id);
    fs::read_to_string(&user_config_path).unwrap_or_default()
}

fn write_user_config(user_id: i64, content: &str) -> std::io::Result<()> {
    let user_config_path = get_user_config_path(user_id);
    fs::write(&user_config_path, content)
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    
    let bot_token = std::env::var("TELEGRAM_BOT_TOKEN")
        .expect("TELEGRAM_BOT_TOKEN environment variable is required. Get it from @BotFather on Telegram.");
    
    let bot = Bot::new(bot_token);
    let processes: ProcessMap = Arc::new(Mutex::new(HashMap::new()));
    
    println!("🤖 Telegram bot starting...");
    
    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message_command))
        .branch(Update::filter_callback_query().endpoint(handle_callback_query));
    
    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![processes])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}

async fn handle_message_command(bot: Bot, msg: Message) -> ResponseResult<()> {
    if let Some(text) = msg.text() {
        match text {
            "/start" | "/menu" => {
                // Initialize user config and log user info
                let user_id = msg.chat.id.0;
                let username = msg.chat.username().map(|s| s.to_string()).unwrap_or_else(|| "N/A".to_string());
                let first_name = msg.chat.first_name().map(|s| s.to_string()).unwrap_or_else(|| "N/A".to_string());
                let last_name = msg.chat.last_name().map(|s| s.to_string()).unwrap_or_else(|| "".to_string());
                
                // Log user info to terminal
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                println!("👤 New User Started Bot");
                println!("   User ID: {}", user_id);
                println!("   Username: @{}", username);
                println!("   Name: {} {}", first_name, last_name);
                println!("   Config: users/{}", user_id);
                println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
                
                // Initialize user config file
                if let Err(e) = initialize_user_config(user_id) {
                    eprintln!("⚠️ Failed to initialize user config for {}: {}", user_id, e);
                } else {
                    println!("✅ User config file initialized: users/{}", user_id);
                }
                
                send_main_menu(&bot, msg.chat.id).await?;
            }
            cmd if cmd.starts_with("/set ") => {
                handle_set_command(&bot, msg.chat.id, cmd).await?;
            }
            _ => {
                bot.send_message(msg.chat.id, "Use /start or /menu to see the main menu.")
                    .await?;
            }
        }
    }
    Ok(())
}

async fn handle_callback_query(
    bot: Bot,
    q: CallbackQuery,
    processes: ProcessMap,
) -> ResponseResult<()> {
    if let Some(data) = q.data {
        if let Some(msg) = q.message {
            let chat_id = msg.chat.id;
            
            match data.as_str() {
                "manage_env" => {
                    handle_manage_env(&bot, chat_id).await?;
                }
                "check" => {
                    handle_validate_setup(&bot, chat_id, processes.clone()).await?;
                }
                "run_auth" => {
                    handle_run_binary(&bot, chat_id, "auth", processes.clone(), msg.id).await?;
                }
                "run_stream" => {
                    handle_run_binary(&bot, chat_id, "stream", processes.clone(), msg.id).await?;
                }
                "run_engine" => {
                    handle_run_binary(&bot, chat_id, "engine", processes.clone(), msg.id).await?;
                }
                "run_test" => {
                    handle_run_binary(&bot, chat_id, "test", processes.clone(), msg.id).await?;
                }
                "run_watch" => {
                    handle_run_binary(&bot, chat_id, "watch", processes.clone(), msg.id).await?;
                }
                cmd if cmd.starts_with("stop_") => {
                    let binary_name = cmd.strip_prefix("stop_").unwrap();
                    handle_stop_process(&bot, chat_id, binary_name, processes.clone()).await?;
                }
                cmd if cmd.starts_with("set_env_") => {
                    let var_name = cmd.strip_prefix("set_env_").unwrap();
                    // Check if it's a boolean variable
                    if var_name == "ENABLE_TRADING" || var_name == "MOCK_TRADING" {
                        handle_prompt_bool_var(&bot, chat_id, var_name).await?;
                    } else {
                        handle_prompt_env_var(&bot, chat_id, var_name).await?;
                    }
                }
                cmd if cmd.starts_with("set_bool_") => {
                    // Format: set_bool_VAR_NAME_true or set_bool_VAR_NAME_false
                    let rest = cmd.strip_prefix("set_bool_").unwrap();
                    if let Some((var_name, value)) = rest.rsplit_once('_') {
                        let bool_value = value == "true";
                        handle_set_bool_var(&bot, chat_id, var_name, bool_value).await?;
                    }
                }
                "back_to_menu" => {
                    send_main_menu(&bot, chat_id).await?;
                }
                _ => {}
            }
        } else {
            return Ok(());
        }
        
        // Answer callback query to remove loading state
        bot.answer_callback_query(q.id).await?;
    }
    Ok(())
}

async fn send_main_menu(bot: &Bot, chat_id: ChatId) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("⚙️ Manage Environment Variables", "manage_env")],
        vec![InlineKeyboardButton::callback("✅ Validate Setup", "check")],
        vec![InlineKeyboardButton::callback("🔐 Approve Tokens", "run_auth")],
        vec![InlineKeyboardButton::callback("⚡ Engine Bot", "run_engine")],
        vec![InlineKeyboardButton::callback("⚡ Stream Bot", "run_stream")],
        vec![
            InlineKeyboardButton::callback("🧪 Test Orders", "run_test"),
            InlineKeyboardButton::callback("📈 Watch Trades", "run_watch")
        ],
    ]);
    
    bot.send_message(chat_id, "🤖 *Polymarket Copy Trading Bot*\n\nSelect an action:")
        .reply_markup(keyboard)
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .await?;
    
    Ok(())
}

async fn handle_manage_env(bot: &Bot, chat_id: ChatId) -> ResponseResult<()> {
    // Only show variables that users need to modify (circled in screenshot)
    // Excluded: CHAINSTACK_API_KEY, ENABLE_PROB_SIZING, CB_*, WS_URL (advanced/internal settings)
    let common_vars = vec![
        "PRIVATE_KEY",
        "FUNDER_ADDRESS",
        "TARGET_WHALE_ADDRESS",
        "ALCHEMY_API_KEY",
        "ENABLE_TRADING",
        "MOCK_TRADING",
        "POSITION_SCALE",
        "BASE_PRICE_BUFFER",
        "MIN_WHALE_SHARES",
        "MIN_TRADE_VALUE_USD",
        "MIN_SHARES",
    ];
    
    // Read current user config file
    let user_id = chat_id.0;
    let current_env = read_user_config(user_id);
    let current_vars: HashMap<String, String> = current_env
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let parts: Vec<&str> = line.splitn(2, '=').collect();
            if parts.len() == 2 {
                Some((parts[0].trim().to_string(), parts[1].trim().to_string()))
            } else {
                None
            }
        })
        .collect();
    
    // Create keyboard with environment variables
    let mut keyboard_buttons: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    
    for var in common_vars.iter() {
        let value = current_vars.get(*var).cloned().unwrap_or_default();
        let display = if value.is_empty() {
            format!("{}: (not set)", var)
        } else if var.contains("KEY") || var.contains("PRIVATE") {
            format!("{}: ***", var)
        } else {
            let truncated = if value.len() > 20 {
                format!("{}...", &value[..20])
            } else {
                value
            };
            format!("{}: {}", var, truncated)
        };
        keyboard_buttons.push(vec![
            InlineKeyboardButton::callback(display, format!("set_env_{}", var))
        ]);
    }
    
    keyboard_buttons.push(vec![InlineKeyboardButton::callback("◀️ Back to Menu", "back_to_menu")]);
    
    let keyboard = InlineKeyboardMarkup::new(keyboard_buttons);
    
    bot.send_message(chat_id, "⚙️ *Environment Variables*\n\nSelect a variable to edit:\n\nUse `/set VAR\\_NAME value` to set a variable\\.")
        .reply_markup(keyboard)
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .await?;
    
    Ok(())
}

async fn handle_prompt_env_var(bot: &Bot, chat_id: ChatId, var_name: &str) -> ResponseResult<()> {
    bot.send_message(
        chat_id,
        format!("📝 To set `{}`, use:\n\n`/set {} your\\_value\\_here`\n\nExample:\n`/set {} abc123`", var_name, var_name, var_name),
    )
    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
    .await?;
    
    Ok(())
}

async fn handle_prompt_bool_var(bot: &Bot, chat_id: ChatId, var_name: &str) -> ResponseResult<()> {
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("✅ True", format!("set_bool_{}_true", var_name)),
            InlineKeyboardButton::callback("❌ False", format!("set_bool_{}_false", var_name)),
        ],
        vec![InlineKeyboardButton::callback("◀️ Back to Menu", "back_to_menu")],
    ]);
    
    bot.send_message(
        chat_id,
        format!("📝 Select value for `{}`:", var_name),
    )
    .reply_markup(keyboard)
    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
    .await?;
    
    Ok(())
}

async fn handle_set_bool_var(bot: &Bot, chat_id: ChatId, var_name: &str, value: bool) -> ResponseResult<()> {
    let user_id = chat_id.0;
    let value_str = if value { "true" } else { "false" };
    
    // Ensure user config exists
    if let Err(e) = initialize_user_config(user_id) {
        bot.send_message(chat_id, format!("❌ Failed to access your config file: {}", e))
            .await?;
        return Ok(());
    }
    
    // Read current user config
    let env_content = read_user_config(user_id);
    let mut lines: Vec<String> = env_content.lines().map(|s| s.to_string()).collect();
    
    // Update or add the variable
    let mut found = false;
    for line in lines.iter_mut() {
        if line.trim().starts_with(&format!("{}=", var_name)) {
            *line = format!("{}={}", var_name, value_str);
            found = true;
            break;
        }
    }
    
    if !found {
        lines.push(format!("{}={}", var_name, value_str));
    }
    
    // ENABLE_TRADING and MOCK_TRADING are always opposite
    // If setting one to true, set the other to false
    // If setting one to false, set the other to true
    if var_name == "ENABLE_TRADING" || var_name == "MOCK_TRADING" {
        let opposite_var = if var_name == "ENABLE_TRADING" {
            "MOCK_TRADING"
        } else {
            "ENABLE_TRADING"
        };
        
        let opposite_value = if value { "false" } else { "true" };
        
        let mut opposite_found = false;
        for line in lines.iter_mut() {
            if line.trim().starts_with(&format!("{}=", opposite_var)) {
                *line = format!("{}={}", opposite_var, opposite_value);
                opposite_found = true;
                break;
            }
        }
        
        if !opposite_found {
            lines.push(format!("{}={}", opposite_var, opposite_value));
        }
    }
    
    // Write back to user config file
    write_user_config(user_id, &lines.join("\n"))
        .map_err(|e| teloxide::RequestError::Io(e))?;
    
    // Automatically go back to Environment Variables menu
    handle_manage_env(bot, chat_id).await?;
    
    Ok(())
}

async fn handle_set_command(bot: &Bot, chat_id: ChatId, cmd: &str) -> ResponseResult<()> {
    let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
    if parts.len() < 3 {
        bot.send_message(chat_id, "❌ Invalid format. Use: `/set VAR_NAME value`")
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .await?;
        return Ok(());
    }
    
    let var_name = parts[1];
    let value = parts[2];
    let user_id = chat_id.0;
    
    // Ensure user config exists
    if let Err(e) = initialize_user_config(user_id) {
        bot.send_message(chat_id, format!("❌ Failed to access your config file: {}", e))
            .await?;
        return Ok(());
    }
    
    // Read current user config
    let env_content = read_user_config(user_id);
    let mut lines: Vec<String> = env_content.lines().map(|s| s.to_string()).collect();
    
    // Update or add the variable
    let mut found = false;
    for line in lines.iter_mut() {
        if line.trim().starts_with(&format!("{}=", var_name)) {
            *line = format!("{}={}", var_name, value);
            found = true;
            break;
        }
    }
    
    if !found {
        lines.push(format!("{}={}", var_name, value));
    }
    
    // ENABLE_TRADING and MOCK_TRADING are always opposite
    // If setting one to true, set the other to false
    // If setting one to false, set the other to true
    if var_name == "ENABLE_TRADING" || var_name == "MOCK_TRADING" {
        let value_lower = value.trim().to_lowercase();
        let is_true = value_lower == "true" || value_lower == "1";
        
        let opposite_var = if var_name == "ENABLE_TRADING" {
            "MOCK_TRADING"
        } else {
            "ENABLE_TRADING"
        };
        
        let opposite_value = if is_true { "false" } else { "true" };
        
        let mut opposite_found = false;
        for line in lines.iter_mut() {
            if line.trim().starts_with(&format!("{}=", opposite_var)) {
                *line = format!("{}={}", opposite_var, opposite_value);
                opposite_found = true;
                break;
            }
        }
        
        if !opposite_found {
            lines.push(format!("{}={}", opposite_var, opposite_value));
        }
    }
    
    // Write back to user config file
    write_user_config(user_id, &lines.join("\n"))
        .map_err(|e| teloxide::RequestError::Io(e))?;
    
    // Automatically go back to Environment Variables menu (removes instruction message)
    handle_manage_env(bot, chat_id).await?;
    
    Ok(())
}

async fn handle_validate_setup(
    bot: &Bot,
    chat_id: ChatId,
    processes: ProcessMap,
) -> ResponseResult<()> {
    // Check if already running
    let procs: tokio::sync::MutexGuard<'_, HashMap<String, (TokioChild, MessageId)>> = processes.lock().await;
    if procs.contains_key("check") {
        bot.send_message(chat_id, "⚠️ Validate setup is already running!")
            .await?;
        return Ok(());
    }
    drop(procs);
    
    let user_id = chat_id.0;
    let user_config_path = get_user_config_path(user_id);
    
    // Ensure user config file exists before validation
    if let Err(e) = initialize_user_config(user_id) {
        bot.send_message(chat_id, format!("❌ Failed to initialize your config file: {}", e))
            .await?;
        return Ok(());
    }
    
    let status_msg = bot.send_message(chat_id, format!("🔄 Validating your configuration (users/{})...", user_id))
        .await?;
    
    // Run check binary from target/release
    let binary_path = std::path::PathBuf::from("target/release/check");
    let mut cmd = TokioCommand::new(&binary_path);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.current_dir(".");
    // Set USER_CONFIG_FILE environment variable so binary loads from user config
    cmd.env("USER_CONFIG_FILE", user_config_path.to_string_lossy().as_ref());
    
    let child = cmd.spawn()
        .map_err(|e| teloxide::RequestError::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("Failed to spawn process: {}", e))))?;
    
    // Store process BEFORE moving child into async task
    let msg_id = status_msg.id;
    {
        let mut procs: tokio::sync::MutexGuard<'_, HashMap<String, (TokioChild, MessageId)>> = processes.lock().await;
        procs.insert("check".to_string(), (child, msg_id));
    }
    
    // Stream output
    let bot_clone = bot.clone();
    let chat_id_clone = chat_id;
    let processes_clone = processes.clone();
    
    tokio::spawn(async move {
        let mut full_log = String::new();
        let mut update_buffer = String::new();
        
        // Get child from processes map
        let mut child_opt = None;
        {
            let mut procs = processes_clone.lock().await;
            if let Some((child, _)) = procs.remove("check") {
                child_opt = Some(child);
            }
        }
        
        if let Some(mut child) = child_opt {
            if let Some(stdout) = child.stdout.take() {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                
                while let Ok(Some(line)) = lines.next_line().await {
                    full_log.push_str(&line);
                    full_log.push('\n');
                    update_buffer.push_str(&line);
                    update_buffer.push('\n');
                    
                    // Send updates every 10 lines or when buffer gets large
                    if update_buffer.lines().count() >= 10 || update_buffer.len() > 1500 {
                        let _ = bot_clone.edit_message_text(
                            chat_id_clone,
                            msg_id,
                            format!("🔄 *Validate Setup*\n```\n{}\n```", full_log.replace('*', "\\*").replace('_', "\\_").replace('[', "\\[").replace(']', "\\]").replace('(', "\\(").replace(')', "\\)")),
                        )
                        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                        .await;
                        update_buffer.clear();
                    }
                }
            }
            
            // Wait for process to finish
            let _ = child.wait().await;
        }
        
        // Show final output with complete log and "Back to Menu" button
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("◀️ Back to Menu", "back_to_menu")
        ]]);
        
        let final_message = if full_log.trim().is_empty() {
            "✅ *Validate Setup Completed\\!*".to_string()
        } else {
            format!("🔄 *Validate Setup*\n```\n{}\n```\n\n✅ *Validation Completed\\!*", full_log.replace('*', "\\*").replace('_', "\\_").replace('[', "\\[").replace(']', "\\]").replace('(', "\\(").replace(')', "\\)"))
        };
        
        let _ = bot_clone.edit_message_text(chat_id_clone, msg_id, final_message)
            .reply_markup(keyboard)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .await;
    });
    
    Ok(())
}

async fn handle_run_binary(
    bot: &Bot,
    chat_id: ChatId,
    binary_name: &str,
    processes: ProcessMap,
    _callback_msg_id: MessageId,
) -> ResponseResult<()> {
    // Check if already running
    let procs: tokio::sync::MutexGuard<'_, HashMap<String, (TokioChild, MessageId)>> = processes.lock().await;
    if procs.contains_key(binary_name) {
        bot.send_message(chat_id, format!("⚠️ {} is already running!", binary_name))
            .await?;
        return Ok(());
    }
    drop(procs);
    
    let user_id = chat_id.0;
    let user_config_path = get_user_config_path(user_id);
    
    // Ensure user config file exists
    if let Err(e) = initialize_user_config(user_id) {
        bot.send_message(chat_id, format!("❌ Failed to initialize your config file: {}", e))
            .await?;
        return Ok(());
    }
    
    let status_msg = bot.send_message(chat_id, format!("🚀 Starting {}...", binary_name))
        .await?;
    
    // Create stop button
    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("🛑 Stop", format!("stop_{}", binary_name))
    ]]);
    
    let status_msg = bot.edit_message_text(
        chat_id,
        status_msg.id,
        format!("🔄 *{}* is running\\.\\.\\.\n\n```\n\n```", binary_name.replace('*', "\\*").replace('_', "\\_")),
    )
    .reply_markup(keyboard)
    .parse_mode(teloxide::types::ParseMode::MarkdownV2)
    .await?;
    
    // Run binary from target/release
    let binary_path = std::path::PathBuf::from(format!("target/release/{}", binary_name));
    let mut cmd = TokioCommand::new(&binary_path);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.current_dir(".");
    // Set USER_CONFIG_FILE environment variable so binary loads from user config
    cmd.env("USER_CONFIG_FILE", user_config_path.to_string_lossy().as_ref());
    
    let child = cmd.spawn()
        .map_err(|e| teloxide::RequestError::Io(std::io::Error::new(std::io::ErrorKind::Other, format!("Failed to spawn process: {}", e))))?;
    
    // Store process BEFORE moving child into async task
    let msg_id = status_msg.id;
    let binary_name_str = binary_name.to_string();
    {
        let mut procs: tokio::sync::MutexGuard<'_, HashMap<String, (TokioChild, MessageId)>> = processes.lock().await;
        procs.insert(binary_name_str.clone(), (child, msg_id));
    }
    
    // Stream output
    let bot_clone = bot.clone();
    let chat_id_clone = chat_id;
    let processes_clone = processes.clone();
    
    tokio::spawn(async move {
        let mut full_log = String::new();
        
        // Get stdout and stderr from child without removing it from the map
        // We need to access the child's streams while keeping it in the map for stop functionality
        let mut stdout_opt = None;
        let mut stderr_opt = None;
        {
            let mut procs = processes_clone.lock().await;
            if let Some((child, _)) = procs.get_mut(&binary_name_str) {
                stdout_opt = child.stdout.take();
                stderr_opt = child.stderr.take();
            }
        }
        
        // Spawn tasks to read both stdout and stderr
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        
        if let Some(stdout) = stdout_opt {
            let tx_clone = tx.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_clone.send(line);
                }
            });
        }
        
        if let Some(stderr) = stderr_opt {
            let tx_clone = tx.clone();
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_clone.send(line);
                }
            });
        }
        
        drop(tx); // Close the sender when both tasks are done
        
        // Helper function to update the message
        let update_message = |log: &str| {
            let keyboard = InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback("🛑 Stop", format!("stop_{}", binary_name_str))
            ]]);
            
            bot_clone.edit_message_text(
                chat_id_clone,
                msg_id,
                format!("🔄 *{}* is running\\.\\.\\.\n\n```\n{}\n```", binary_name_str.replace('*', "\\*").replace('_', "\\_"), log.replace('*', "\\*").replace('_', "\\_").replace('[', "\\[").replace(']', "\\]").replace('(', "\\(").replace(')', "\\)")),
            )
            .reply_markup(keyboard)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        };
        
        // Read from both streams with real-time updates (update every 200ms or immediately on new line)
        let mut last_update = std::time::Instant::now();
        let update_interval = std::time::Duration::from_millis(200); // Update every 200ms for real-time feel
        let mut pending_update = false;
        
        loop {
            // Check if process is still in map (hasn't been stopped)
            let still_running = {
                let procs = processes_clone.lock().await;
                procs.contains_key(&binary_name_str)
            };
            
            if !still_running {
                // Process was stopped, send final update if there's pending content
                if pending_update && !full_log.is_empty() {
                    let _ = update_message(&full_log).await;
                }
                break;
            }
            
            // Try to read next line with shorter timeout for responsiveness
            match tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await {
                Ok(Some(line)) => {
                    full_log.push_str(&line);
                    full_log.push('\n');
                    pending_update = true;
                    
                    // Update immediately if enough time has passed since last update
                    if last_update.elapsed() >= update_interval {
                        let _ = update_message(&full_log).await;
                        last_update = std::time::Instant::now();
                        pending_update = false;
                    }
                }
                Ok(None) => {
                    // Channel closed - process finished, send final update
                    if !full_log.is_empty() {
                        let _ = update_message(&full_log).await;
                    }
                    break;
                }
                Err(_) => {
                    // Timeout - if we have pending updates and enough time passed, update now
                    if pending_update && last_update.elapsed() >= update_interval && !full_log.is_empty() {
                        let _ = update_message(&full_log).await;
                        last_update = std::time::Instant::now();
                        pending_update = false;
                    }
                    continue;
                }
            }
        }
        
        // Wait for process to finish (if it hasn't been stopped already)
        {
            let mut procs = processes_clone.lock().await;
            if let Some((mut child, _)) = procs.remove(&binary_name_str) {
                // Process still in map, wait for it to finish
                drop(procs); // Release lock before waiting
                let _ = child.wait().await;
            }
        }
        
        // Show final output with complete log
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("◀️ Back to Menu", "back_to_menu")
        ]]);
        
        let final_message = if full_log.trim().is_empty() {
            format!("✅ *{}* has stopped\\.", binary_name_str.replace('*', "\\*").replace('_', "\\_"))
        } else {
            format!("🔄 *{}* has stopped\\.\\.\\.\n\n```\n{}\n```", binary_name_str.replace('*', "\\*").replace('_', "\\_"), full_log.replace('*', "\\*").replace('_', "\\_").replace('[', "\\[").replace(']', "\\]").replace('(', "\\(").replace(')', "\\)"))
        };
        
        let _ = bot_clone.edit_message_text(
            chat_id_clone,
            msg_id,
            final_message,
        )
        .reply_markup(keyboard)
        .parse_mode(teloxide::types::ParseMode::MarkdownV2)
        .await;
    });
    
    Ok(())
}

async fn handle_stop_process(
    bot: &Bot,
    chat_id: ChatId,
    binary_name: &str,
    processes: ProcessMap,
) -> ResponseResult<()> {
    let mut procs: tokio::sync::MutexGuard<'_, HashMap<String, (TokioChild, MessageId)>> = processes.lock().await;
    
    if let Some(process_tuple) = procs.remove(binary_name) {
        drop(procs); // Release lock before killing (kill might take time)
        let (mut child, msg_id): (TokioChild, MessageId) = process_tuple;
        
        // Kill the process (start_kill sends kill signal, then wait for termination)
        if let Err(e) = child.start_kill() {
            bot.send_message(chat_id, format!("⚠️ Failed to stop *{}*: {}", binary_name.replace('*', "\\*").replace('_', "\\_"), e))
                .parse_mode(teloxide::types::ParseMode::MarkdownV2)
                .await?;
            return Ok(());
        }
        
        // Wait for process to terminate
        let _ = child.wait().await;
        
        // Wait for process to terminate
        let _ = child.wait().await;
        
        // Update status message with "Back to Menu" button
        let keyboard = InlineKeyboardMarkup::new(vec![vec![
            InlineKeyboardButton::callback("◀️ Back to Menu", "back_to_menu")
        ]]);
        
        bot.edit_message_text(chat_id, msg_id, format!("🛑 *{}* stopped by user\\.", binary_name.replace('*', "\\*").replace('_', "\\_")))
            .reply_markup(keyboard)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .await?;
    } else {
        bot.send_message(chat_id, format!("⚠️ *{}* is not running\\.", binary_name.replace('*', "\\*").replace('_', "\\_")))
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .await?;
    }
    
    Ok(())
}
