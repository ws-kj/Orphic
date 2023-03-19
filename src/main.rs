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
use std::fmt;

pub mod prompts;

#[derive(Debug)]
struct UserAbort();

#[derive(Debug, Copy, Clone)]
struct Flags {
    repl:        bool,
    interpret:   bool,
    debug:       bool,
    unsafe_mode: bool
}

impl fmt::Display for UserAbort {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "User aborted command")
    }
}
impl Error for UserAbort {}

fn get_prompt(key: &'static str) -> &str {
    assert!(prompts::PROMPTS[key].is_string());
    prompts::PROMPTS[key].as_str().unwrap()
}

fn try_extract(body: &String) -> Option<Value> {
    if body.find('{') == None || body.find('}') == None {
        return None;
    }

    let data = body.substring(body.find('{').unwrap(),body.rfind('}').unwrap()+1); 
    
    match serde_json::from_str(&data) {
        Ok(commands) => Some(commands),
        Err(e) => { println!("{}", e); None }
    }
}

async fn parse_command(client: &Client, body: &String) -> Result<Option<Value>, Box<dyn Error>> {
    match try_extract(body) {
        Some(commands) => Ok(Some(commands)),
        None => {
            match verify_json(client, body).await? {
                Some(body) => Ok(try_extract(&body)),
                None => Ok(None)
            }
        }
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

async fn interpret(client: &Client, task: &String, output: &String) -> Result<String, Box<dyn Error>> {
    let request = CreateChatCompletionRequestArgs::default()
        .max_tokens(512u16)
        .model("gpt-3.5-turbo")
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

async fn try_command(client: &Client, input: String, history: &mut Vec<ChatCompletionRequestMessage>, flags: Flags) -> Result<String, Box<dyn Error>> {
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

    if flags.debug && flags.unsafe_mode { println!("{}", body); }

    return match parse_command(client, &body).await? {
        Some(commands) => {
            match commands["command"].as_str() {
                Some(command) => {
                    if !flags.unsafe_mode {
                        let mut input = String::new();
                        println!("{}", command);
                        print!("Execute? [Y/n] ");
                        io::stdout().flush()?;
                        io::stdin().read_line(&mut input)?;

                        match input.trim().to_lowercase().as_str() {
                            ""  | "y" | "yes" => {},
                            _ => return Err(Box::new(UserAbort()))
                        };
                    }

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
    
async fn repl(client: &Client, flags: Flags) -> Result<(), Box<dyn Error>> {
    let mut history: Vec<ChatCompletionRequestMessage> = Vec::new();

    loop {
       let mut input = String::new();
       print!("orphic> ");
       io::stdout().flush()?;
       io::stdin().read_line(&mut input)?;
       match input.as_str().trim() {
            "quit" => break,
            task => {
                let res_maybe = try_command(client, String::from(task), &mut history, flags).await;
                match res_maybe {
                    Ok(res) => {
                        history.push(ChatCompletionRequestMessage {
                            role: Role::Assistant,
                            content: res.clone(),
                            name: None
                        });
                        
                        if flags.interpret {
                            println!("{}", interpret(&client, &(String::from(task.trim())), &res).await?);
                        } else {
                            println!("{}", res.trim());
                        }
                    },
                    Err(error) => { 
                        if error.is::<UserAbort>() {
                            continue;
                        } else {
                            return Err(error);
                        }
                    }
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
        .arg(
            Arg::new("unsafe")
            .short('u')
            .long("unsafe")
            .action(ArgAction::SetTrue)
            .help("Execute commands without confirmation")
        )
        .get_matches();

    let flags = Flags {
        repl:        matches.get_flag("repl"),
        interpret:   matches.get_flag("interpret"),
        debug:       matches.get_flag("debug"),
        unsafe_mode: matches.get_flag("unsafe")
    };

    let client = Client::new();

    if flags.repl {
        repl(&client, flags).await?;
        return Ok(());
    }

    let task = matches
        .get_many::<String>("task")
        .unwrap_or_default()
        .map(|v| v.as_str())
        .collect::<Vec<_>>();

    let mut history: Vec<ChatCompletionRequestMessage> = Vec::new();

    let res = try_command(&client, task.join(" "), &mut history, flags).await?;

    if flags.interpret {
        println!("{}", interpret(&client, &(task.join(" ")), &res).await?);
    } else {
        println!("{}", res.trim());
    }

    Ok(())
}
