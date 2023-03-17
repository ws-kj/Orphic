#[allow(unused, dead_code)]

use async_openai::{
    types::{ 
        CreateChatCompletionRequestArgs,
        ChatCompletionRequestMessage,
        Role 
    },  Client
};
use serde_json::Value;
use substring::Substring;
use execute::{Execute, shell};
use clap::{command, Arg, ArgAction};

use std::error::Error;
use std::process::Stdio;
use std::io::{self, Write};

pub mod prompts;

fn get_prompt(key: &'static str) -> &str {
    assert!(prompts::PROMPTS[key].is_string());
    prompts::PROMPTS[key].as_str().unwrap()
}

fn parse_command(body: &String, tried_verify: bool) -> Option<Value> {
    if body.find('{') == None || body.find('}') == None {
        return None;
    }

    let data = body.substring(body.find('{').unwrap(),body.find('}').unwrap()+1);
    match serde_json::from_str(data) {
        Ok(commands) => Some(commands), //todo gpt verify
        Err(_) => None
    }
}

async fn verify_json(client: &Client, input: &String) -> Result<Option<String>, Box<dyn Error>> {
    let history = vec![
        ChatCompletionRequestMessage {
            role: Role::System,
            content: String::from(get_prompt("json_verify_system")),
            name: None
        },
        ChatCompletionRequestMessage {
            role: Role::User,
            content: String::from(get_prompt("json_verify_user")) + input,
            name: None
        }
    ];

    let request = CreateChatCompletionRequestArgs::default()
        .max_tokens(512u16)
        .model("gpt-3.5-turbo")
        .messages(history)
        .build()?;

    let response = client.chat().create(request).await?;
    let body = (response.choices[0]).message.content.to_owned();
    
    return match body.trim() {
        "" => Ok(None),
        _ => Ok(Some(body))
    }
}

async fn try_command(client: &Client, input: String, history: &mut Vec<ChatCompletionRequestMessage>) -> Result<String, Box<dyn Error>> {
    history.push(ChatCompletionRequestMessage {
        role: Role::User,
        content: input + get_prompt("assistant_user"),
        name: None
    });

    let request = CreateChatCompletionRequestArgs::default()
        .max_tokens(512u16)
        .model("gpt-3.5-turbo")
        .messages((*history).clone())
        .build()?;

    let response = client.chat().create(request).await?;
    let body = (response.choices[0]).message.content.to_owned();

    return match parse_command(&body, false) {
        Some(commands) => {
            match commands["command"].as_str() {
                Some(command) => {
                    let mut shell = shell(command);
                    shell.stdout(Stdio::piped());
                    let output = shell.execute_output()?;
                    let out = String::from_utf8(output.stdout)?;
                    Ok(out)

                },
                None => Ok(body)
            }
        },
        None => Ok(body)
    }
}
    
async fn repl(client: &Client) -> Result<(), Box<dyn Error>> {
    let mut history: Vec<ChatCompletionRequestMessage> = Vec::new();

    // assistant system
    // assistant examples
    
    loop {
       let mut input = String::new();
       print!("orphic> ");
       io::stdout().flush()?;
       io::stdin().read_line(&mut input)?;
       match input.as_str().trim() {
            "quit" => break,
            _ => {
                let resp = try_command(client, input, &mut history).await?;
                history.push(ChatCompletionRequestMessage {
                    role: Role::Assistant,
                    content: resp,
                    name: None
                });
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let matches = command!()
        .arg(Arg::new("task").action(ArgAction::Append))
        .arg(
            Arg::new("repl")
            .short('r')
            .long("repl")
            .action(ArgAction::SetTrue)
        )
        .get_matches();

    let client = Client::new();

    if matches.get_flag("repl") {
        repl(&client).await?;
        return Ok(());
    }

    let task = matches
        .get_many::<String>("task")
        .unwrap_or_default()
        .map(|v| v.as_str())
        .collect::<Vec<_>>();

    let mut history: Vec<ChatCompletionRequestMessage> = Vec::new();
    let res = try_command(&client, task.join(" "), &mut history).await?;
    println!("{}", res);
    
    Ok(())
}
