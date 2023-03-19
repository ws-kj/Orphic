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
use serde_json::json;

use std::error::Error;
use std::process::Stdio;
use std::io::{self, Write};
// use std::env::consts::OS;

pub mod prompts;

const GPT35TURBO: &str = "gpt-3.5-turbo";
const GPT4: &str = "gpt-4";

fn get_prompt(key: &'static str) -> &str {
    assert!(prompts::PROMPTS[key].is_string());
    prompts::PROMPTS[key].as_str().unwrap()
}

fn try_extract(body: &String) -> Option<Value> {
    if body.find('{') == None || body.find('}') == None {
        return None;
    }

    let data = body.substring(body.find('{').unwrap(),body.find('}').unwrap()+1); 
   //     .replace("*", "\\*");
    //println!("{}", data);
    match serde_json::from_str(&data) {
        Ok(commands) => Some(commands),
        Err(e) => { println!("{}", e); None }
    }
}

async fn parse_command(client: &Client, model: &str, body: &String) -> Result<Option<Value>, Box<dyn Error>> {
    match try_extract(body) {
        Some(commands) => Ok(Some(commands)),
        None => {
            match verify_json(client, model, body).await? {
                Some(body) => Ok(try_extract(&body)),
                None => Ok(None)
            }
        }
    }
}

async fn verify_json(client: &Client, model: &str, input: &String) -> Result<Option<String>, Box<dyn Error>> {
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
        .model(model)
        .messages(history)
        .build()?;

    let response = client.chat().create(request).await?;
    let body = (response.choices[0]).message.content.to_owned();
    
    return match body.trim() {
        "" => Ok(None),
        _ => Ok(Some(body))
    }
}

async fn interpret(client: &Client, model: &str, task: &String, output: &String) -> Result<String, Box<dyn Error>> {
    let request = CreateChatCompletionRequestArgs::default()
        .max_tokens(512u16)
        .model(model)
        .messages(vec![
            ChatCompletionRequestMessage {
                role: Role::System,
                content: String::from(get_prompt("interpreter_system")),
                name: None
            },
            ChatCompletionRequestMessage {
                role: Role::User,
                content: String::from(json!({"task": task, "output": output}).to_string()) + get_prompt("interpreter_user"),
                name: None
            },
        ])
        .build()?;

    let response = client.chat().create(request).await?;
    Ok((response.choices[0]).message.content.to_owned())
}

async fn try_command(client: &Client, model: &str, input: String, history: &mut Vec<ChatCompletionRequestMessage>, verbose: bool) -> Result<String, Box<dyn Error>> {
    history.push(ChatCompletionRequestMessage {
        role: Role::User,
        content: input + get_prompt("assistant_user"),
        name: None
    });

    let request = CreateChatCompletionRequestArgs::default()
        .max_tokens(512u16)
        .model(model)
        .messages((*history).clone())
        .build()?;

    let response = client.chat().create(request).await?;
    let body = (response.choices[0]).message.content.to_owned();

    if verbose { println!("{}", body); }

    return match parse_command(client, model, &body).await? {
        Some(commands) => {
            match commands["command"].as_str() {
                Some(command) => {
                    let mut shell = shell(command);
                    shell.stdout(Stdio::piped());
                    Ok(String::from_utf8(shell.execute_output()?.stdout)?)

                },
                None => Ok(body)
            }
        },
        None => Ok(body)
    }
}
    
async fn repl(client: &Client, model: &str, do_interpret: bool, verbose: bool) -> Result<(), Box<dyn Error>> {
    let mut history: Vec<ChatCompletionRequestMessage> = Vec::new();

    loop {
       let mut input = String::new();
       print!("orphic> ");
       io::stdout().flush()?;
       io::stdin().read_line(&mut input)?;
       match input.as_str().trim() {
            "quit" => break,
            task => {
                let res = try_command(client, model, String::from(task), &mut history, verbose).await?;
                history.push(ChatCompletionRequestMessage {
                    role: Role::Assistant,
                    content: res.clone(),
                    name: None
                });
                
                if do_interpret {
                    println!("{}", interpret(&client, model, &(String::from(task.trim())), &res).await?);
                } else {
                    println!("{}", res.trim());
                }
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
            .help("Start a REPL environment for orphic commands")
        )
        .arg(
            Arg::new("interpret")
            .short('i')
            .long("interpret")
            .action(ArgAction::SetTrue)
            .help("Interpret output into natural language")
        )
        .arg(
            Arg::new("debug")
            .short('d')
            .long("verbose")
            .action(ArgAction::SetTrue)
            .help("Display raw GPT output")
        )
        .get_matches();

    let use_gpt4: bool = option_env!("USE_GPT4").unwrap_or("false").eq("true");
    let model = if use_gpt4 { GPT4 } else { GPT35TURBO };

    let client = Client::new();

    if matches.get_flag("repl") {
        repl(&client, model, matches.get_flag("interpret"), matches.get_flag("debug")).await?;
        return Ok(());
    }

    let task = matches
        .get_many::<String>("task")
        .unwrap_or_default()
        .map(|v| v.as_str())
        .collect::<Vec<_>>();

    let mut history: Vec<ChatCompletionRequestMessage> = Vec::new();

    let res = try_command(&client, model, task.join(" "), &mut history, matches.get_flag("debug")).await?;

    if matches.get_flag("interpret") {
        println!("{}", interpret(&client, model, &(task.join(" ")), &res).await?);
    } else {
        println!("{}", res.trim());
    }

    Ok(())
}
