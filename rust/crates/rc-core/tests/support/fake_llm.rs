//! Fake `claude.exe` / `codex.exe` for the LLM tests. Echoes what it received
//! (args, stdin, MAX_THINKING_TOKENS) back as the model's JSON answer.
//! Behaviour switches on words in the prompt: FAIL, SLEEP, GARBAGE, CODEFAIL.

use std::io::{Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("auth") {
        println!("{{\"loggedIn\": true, \"authMethod\": \"claude.ai\"}}");
        return;
    }
    let mut stdin = String::new();
    std::io::stdin().read_to_string(&mut stdin).unwrap();
    let echo = serde_json::json!({
        "args": args,
        "stdin": stdin,
        "thinking": std::env::var("MAX_THINKING_TOKENS").ok(),
    });
    if stdin.contains("SLEEP") {
        std::thread::sleep(std::time::Duration::from_secs(10));
    }
    if args.first().map(String::as_str) == Some("exec") {
        if stdin.contains("CODEFAIL") {
            eprintln!("boom: model not available");
            std::process::exit(3);
        }
        let i = args.iter().position(|a| a == "-o").expect("-o");
        std::fs::write(&args[i + 1], format!("Odpowiedź:\n{echo}\n")).unwrap();
        println!("{{\"type\":\"progress\"}}");
        return;
    }
    let out = if stdin.contains("FAIL") {
        serde_json::json!({ "is_error": true, "result": "limit wyczerpany" }).to_string()
    } else if stdin.contains("GARBAGE") {
        "to nie jest JSON".to_string()
    } else {
        serde_json::json!({ "is_error": false, "result": format!("```json\n{echo}\n```"), "usage": { "input_tokens": 1 } }).to_string()
    };
    std::io::stdout().write_all(out.as_bytes()).unwrap();
}
