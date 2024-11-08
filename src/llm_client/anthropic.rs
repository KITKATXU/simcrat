use std::{
//     fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Instant,
};

// use std::error::Error;
// use anthropic::client::ClientBuilder;
use anthropic::client::Client as AnthropicClient;
use anthropic::config::AnthropicConfig;
use anthropic::types::{ContentBlock, Message, MessagesRequestBuilder, Role};
// use anthropic::types::CompleteRequestBuilder;
// use anthropic::{AI_PROMPT, HUMAN_PROMPT};
use dotenv::dotenv;
use async_trait::async_trait;
use etrace::some_or;
use serde::{Deserialize, Serialize};
// use async_openai::types::*;
// use std::sync::atomic::{AtomicUsize, Ordering};
// use lazy_static::lazy_static;
//
// lazy_static! {
//     // 静态变量用于记录总请求 token 数目
//     static TOTAL_REQUEST_TOKENS: AtomicUsize = AtomicUsize::new(0);
//     // 静态变量用于记录总响应 token 数目
//     static ref TOTAL_RESPONSE_TOKENS: AtomicUsize = AtomicUsize::new(0);
// }

use super::{
    cache::{Cache, DbConfig, HasElapsed},
    tokens_in_str, LanguageModel,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheKey {
    messages: Vec<(String, String)>,
    stop: Option<String>,
}

impl CacheKey {
    fn new<S: AsRef<str>>(messages: &[Message], stop: &Option<S>) -> Self {
        let messages = messages
            .iter()
            .map(|Message { role, content, .. }| {
                let blocks: Vec<ContentBlock> = content.clone(); // 假设这是您从 response 中获得的内容

                // 提取所有文本内容并合并为一个 String
                let content_string: String = blocks.iter()
                    .filter_map(|block| {
                        if let ContentBlock::Text { text } = block {
                            Some(text.clone()) // 提取并克隆文本
                        } else {
                            None // 忽略非 Text 类型的块
                        }
                    })
                    .collect::<Vec<String>>()
                    .join(" "); // 用空格连接所有文本
                (role_to_str(&role).to_string(), content_string)
            })
            .collect();
        let stop = stop.as_ref().map(|s| s.as_ref().to_string());
        Self { messages, stop }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheVal {
    content: String,
    reason: Option<String>,
    response_tokens: usize,
    request_tokens: usize,
    elapsed: f32,
}

impl CacheVal {
    fn new(
        content: String,
        reason: Option<String>,
        response_tokens: usize,
        request_tokens: usize,
        elapsed: f32,
    ) -> Self {
        Self {
            content,
            reason,
            response_tokens,
            request_tokens,
            elapsed,
        }
    }

//     fn is_too_long(&self) -> bool {
//         self.reason.as_ref().map(|s| s == "length").unwrap_or(false)
//     }
}

impl HasElapsed for CacheVal {
    fn elapsed(&self) -> f32 {
        self.elapsed
    }
}


// const ERROR_MESSAGE: &str = "Sorry! We've encountered an issue with repetitive patterns in your prompt. Please try again with a different prompt.";

pub struct AnthroClient {
    inner: Option<AnthropicClient>, // 使用 Anthropic 的客户端
    model: Option<String>,
    cache: Cache<CacheKey, CacheVal>,
    possible_requests: AtomicUsize,

    total_request_tokens: AtomicUsize,
    total_response_tokens: AtomicUsize,
    total_response_time: Mutex<f32>,
}

impl AnthroClient {
    pub fn new(model: Option<String>, _api_key_file: Option<String>, db_conf: DbConfig) -> Self {
//         fn flatten_nested_option<T>(nested: Option<Option<T>>) -> Option<T> {
//             nested.and_then(|inner| inner) // Flatten the nested Option
//         }
        dotenv().ok();
        let cfg = match AnthropicConfig::new() {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("Error creating AnthropicConfig: {}", e);
                panic!("Failed to create AnthropicConfig");
            }
        };
        let inner = match AnthropicClient::try_from(cfg){
                    Ok(client) => Some(client), // Wrap the client in Some if successful
                    Err(e) => {
                        eprintln!("Error creating client: {}", e); // Handle error appropriately
                        None // Return None if there's an error
                    }
                };
//             let api_key = fs::read_to_string(api_key_file).unwrap().trim().to_string();
//             AnthropicClient::new(api_key) // 创建 Anthropic 客户端

//             match ClientBuilder::default()
//                     .api_key(api_key)
//                     .build() // This returns a Result<Client, _>
//                 {
//                     Ok(client) => Some(client), // Wrap the client in Some if successful
//                     Err(e) => {
//                         eprintln!("Error creating client: {}", e); // Handle error appropriately
//                         None // Return None if there's an error
//                     }
//                 }

        let cache = Cache::new(db_conf);
//         let inner = flatten_nested_option(inner_option);
        Self {
            inner,
            model,
            cache,
            possible_requests: AtomicUsize::new(30),
            total_request_tokens: AtomicUsize::new(0),
            total_response_tokens: AtomicUsize::new(0),
            total_response_time: Mutex::new(0.0),
        }
    }

    async fn send_request(
        &self,
        mut msgs: Vec<Message>,
        stop: Option<&str>,
    ) -> String {



        let msgs_str = msgs
            .iter()
            .map(|msg| {
                let msg_blocks: Vec<ContentBlock> = msg.content.clone(); // 假设这是您从 response 中获得的内容

                // 提取所有文本内容并合并为一个 String
                let msg_content: String = msg_blocks.iter()
                    .filter_map(|block| {
                        if let ContentBlock::Text { text } = block {
                            Some(text.clone()) // 提取并克隆文本
                        } else {
                            None // 忽略非 Text 类型的块
                        }
                    })
                    .collect::<Vec<String>>()
                    .join(" "); // 用空格连接所有文本
                format!("{}: {}", role_to_str(&msg.role), msg_content)
            })
            .collect::<Vec<_>>()
            .join("\n");

        let key = CacheKey::new(&msgs, &stop);
        let (result, hit) = if let Some(result) = self.cache.get(&key).await {
            (result, true)
        } else {
            let inner = self.inner.as_ref().expect(&msgs_str);
            let model = self.model.as_ref().expect(&msgs_str);
            let mut i = 0;
            let max_tokens: usize = 4096; // Directly defined as usize
            tracing::info!("send_request START");
            let (response, elapsed) = loop {
                assert!(i < 10, "{:?}", msgs);

                let messages =
                    vec![Message { role: Role::User, content: vec![ContentBlock::Text { text: msgs_str.clone() }] }];

                let messages_request = match MessagesRequestBuilder::default()
                    .messages(messages.clone())
                    .model(model)
                    .max_tokens(max_tokens)
                    .build(){
                        Ok(request) => request,
                        Err(e) => {
                            eprintln!("Failed to build request: {}", e);
                            return String::new(); // Handle the error case accordingly
                        }
                    };

                tracing::info!("send_request trial {}", i + 1);

                let now = Instant::now();
                let response = inner.messages(messages_request).await;
//                 let response = inner.complete(complete_request).await;
                let elapsed = now.elapsed().as_secs_f32();

//                 TOTAL_REQUEST_TOKENS.fetch_add(num_tokens(messages_request), Ordering::Relaxed);
//                 TOTAL_RESPONSE_TOKENS.fetch_add(num_tokens(response), Ordering::Relaxed);

                self.possible_requests.fetch_add(1, Ordering::AcqRel);

                match response {
                        Ok(response) => {
                            tracing::info!(
                                "send_request success at trial {} ({} seconds)",
                                i + 1,
                                elapsed
                            );
                            break (response, elapsed);
                        }
                        Err(err) => {
                            tracing::info!(
                                "send_request failure at trial {} ({} seconds)\n{:?}",
                                i + 1,
                                elapsed,
                                err
                            );

                            if i == 9 {
                                // 处理错误
                            }
//                             msgs.first_mut().unwrap().content += " ";
                            let Some(first_msg) = msgs.first_mut() else { todo!() };
                            if let Some(ContentBlock::Text { ref mut text }) = first_msg.content.first_mut() {
                                    // 将空格追加到现有文本中
                                    *text += " ";
                                }
                            i += 1;
                        }
                }
//                 break (response, elapsed);
            };
            tracing::info!("send_request DONE");

            let content_blocks: Vec<ContentBlock> = response.content.clone(); // 假设这是您从 response 中获得的内容

            // 提取所有文本内容并合并为一个 String
            let content: String = content_blocks.iter()
                .filter_map(|block| {
                    if let ContentBlock::Text { text } = block {
                        Some(text.clone()) // 提取并克隆文本
                    } else {
                        None // 忽略非 Text 类型的块
                    }
                })
                .collect::<Vec<String>>()
                .join(" "); // 用空格连接所有文本
            let reason = "No stop reason".to_string();
            let usage = response.usage;
            let request_tokens = usage.input_tokens;
            let response_tokens = usage.output_tokens;
            let val = CacheVal::new(
                content,
                Some(reason),
                request_tokens,
                response_tokens,
                elapsed,
            );
            self.cache.insert(key, val.clone()).await;
            (val, false)
        };

        tracing::info!(
            "send_request cache {} {}",
            if hit { "hit" } else { "miss" },
            msgs_str
        );

        self.total_request_tokens
            .fetch_add(result.request_tokens, Ordering::AcqRel);
        self.total_response_tokens
            .fetch_add(result.response_tokens, Ordering::AcqRel);
        let mut time = self.total_response_time.lock().unwrap();
        *time += result.elapsed;

        result.content
    }
}

#[async_trait]
impl LanguageModel for AnthroClient {
    fn request_tokens(&self) -> usize {
        self.total_request_tokens.load(Ordering::Acquire)
    }

    fn response_tokens(&self) -> usize {
        self.total_response_tokens.load(Ordering::Acquire)
    }

    fn response_time(&self) -> f32 {
        *self.total_response_time.lock().unwrap()
    }

    async fn rename_type(&self, name: &str) -> String {
        if name.chars().next().unwrap().is_uppercase()
            && !name.contains('_')
            && name.contains(|c: char| c.is_lowercase())
        {
            return name.to_string();
        }
        let m1 = system("You are a helpful assistant. Answer as concisely as possible.");
        let m2 = user("Convert `foo` to `CamelCase`.");
        let m3 = assistant("`Foo`");
        let m4 = user("Convert `Bar` to `CamelCase`.");
        let m5 = assistant("`Bar`");
        let m6 = user("Convert `foo_bar` to `CamelCase`.");
        let m7 = assistant("`FooBar`");
        let m8 = user("Convert `barBaz` to `CamelCase`.");
        let m9 = assistant("`BarBaz`");
        let prompt = format!("Convert `{}` to `CamelCase`.", name);
        let m10 = user(&prompt);
        let msgs = vec![m1, m2, m3, m4, m5, m6, m7, m8, m9, m10];
        let result = self.send_request(msgs, None).await;
        extract_name(result)
    }

    async fn translate_type(&self, code: &str, sort: &str, deps: &[String]) -> Option<String> {
        let m1 = system("You are a helpful assistant that translates C to Rust.");
        let deps = make_deps(deps);
        let prompt = format!(
            "{}Translate the following C {} definition to Rust using Rust idioms without any explanation:
```
{}
```
Try to avoid unsafe code.",
            deps, sort, code
        );
        let m2 = user(&prompt);
        let msgs = vec![m1, m2];
        let result = self.send_request(msgs, None).await;
        extract_code(&result, &["type ", "struct ", "union ", "enum "])
    }

    async fn rename_variable(&self, name: &str) -> String {
        if !name.contains(|c: char| c.is_lowercase()) {
            return name.to_string();
        }
        let m1 = system("You are a helpful assistant. Answer as concisely as possible.");
        let m2 = user("Convert `Foo` to `SCREAMING_SNAKE_CASE`.");
        let m3 = assistant("`FOO`");
        let m4 = user("Convert `BAR` to `SCREAMING_SNAKE_CASE`.");
        let m5 = assistant("`BAR`");
        let m6 = user("Convert `foo_bar` to `SCREAMING_SNAKE_CASE`.");
        let m7 = assistant("`FOO_BAR`");
        let m8 = user("Convert `barBaz` to `SCREAMING_SNAKE_CASE`.");
        let m9 = assistant("`BAR_BAZ`");
        let prompt = format!("Convert `{}` to `SCREAMING_SNAKE_CASE`.", name);
        let m10 = user(&prompt);
        let msgs = vec![m1, m2, m3, m4, m5, m6, m7, m8, m9, m10];
        let result = self.send_request(msgs, None).await;
        extract_name(result).replace(' ', "_")
    }

    async fn translate_variable(&self, code: &str, deps: &[String]) -> Option<String> {
        let m1 = system("You are a helpful assistant that translates C to Rust.");
        let deps = make_deps(deps);
        let prompt = format!(
            "{}Translate the following C global variable declaration to a Rust global variable declaration without any explanation:
```
{}
```
Try to avoid unsafe code.",
            deps, code
        );
        let m2 = user(&prompt);
        let msgs = vec![m1, m2];
        let result = self.send_request(msgs, None).await;
        extract_code(&result, &["const ", "static "])
    }

    async fn rename_function(&self, name: &str) -> String {
        if !name.contains(|c: char| c.is_uppercase()) {
            return name.to_string();
        }
        let m1 = system("You are a helpful assistant. Answer as concisely as possible.");
        let m2 = user("Convert `Foo` to `snake_case`.");
        let m3 = assistant("`foo`");
        let m4 = user("Convert `BAR` to `snake_case`.");
        let m5 = assistant("`bar`");
        let m6 = user("Convert `foo_bar` to `snake_case`.");
        let m7 = assistant("`foo_bar`");
        let m8 = user("Convert `barBaz` to `snake_case`.");
        let m9 = assistant("`bar_baz`");
        let prompt = format!("Convert `{}` to `snake_case`.", name);
        let m10 = user(&prompt);
        let msgs = vec![m1, m2, m3, m4, m5, m6, m7, m8, m9, m10];
        let result = self.send_request(msgs, None).await;
        extract_name(result)
    }

    async fn translate_signature(
        &self,
        code: &str,
        new_name: &str,
        deps: &[String],
        n: usize,
    ) -> Vec<String> {
        assert!((1..=9).contains(&n));
        let m1 = system("You are a helpful assistant.");
        let m2 = user(&signature_prompt(
            "int hello() {
    if (NAME == NULL) {
        return 1;
    }
    printf(\"Hello %s!\\n\", NAME);
    return 0;
}",
            "hello",
            &["const NAME: &str;".to_string()],
            3,
        ));
        let m3 = assistant(
            "Explanation:
The function checks if the global constant `NAME` is `NULL` and returns `1` if it is. \
Otherwise, it prints a greeting message and returns `0`.
Signatures:
1. `fn hello() -> i32;`
2. `fn hello() -> Option<()>;`
3. `fn hello() -> Result<(), ()>;`",
        );
        let m4 = user(&signature_prompt(
            "int divide(int n, int d, int *q, int *r) {
    if (d == 0) {
        return DIV_BY_ZERO;
    }
    *q = n / d;
    *r = n % d;
    return 0;
}",
            "divide",
            &["const DIV_BY_ZERO: i32;".to_string()],
            3,
        ));
        let m5 = assistant(
            "Explanation:
The function takes in two integers and two pointers to integers. \
It checks if the second integer is zero, and if so, returns an error code. \
Otherwise, it calculates the quotient and remainder of the division of the first integer by the second integer \
and stores them in the memory locations pointed to by the two pointers. \
Finally, it returns zero to indicate success.
Signatures:
1. `fn divide(n: i32, d: i32, q: &mut i32, r: &mut i32) -> i32;`
2. `fn divide(n: i32, d: i32) -> Option<(i32, i32)>;`
3. `fn divide(n: i32, d: i32) -> Result<(i32, i32), ()>;`"
        );
        let m6 = user(&signature_prompt(code, new_name, deps, n));
        let msgs = vec![m1, m2, m3, m4, m5, m6];
        let result = self.send_request(msgs, None).await;
        let sigs: Vec<_> = result
            .lines()
            .filter_map(|s| {
                let mut chars = s.chars();
                let c1 = chars.next()?;
                if !('1'..='9').contains(&c1) {
                    return None;
                }
                let c2 = chars.next()?;
                if c2 != '.' {
                    return None;
                }
                let i = s.find('`')?;
                let s = &s[i + 1..];
                let i = s.find('`')?;
                let s = s[..i].trim();
                let s = s.strip_prefix("unsafe ").unwrap_or(s).trim();
                let s = s.strip_suffix(';').unwrap_or(s).trim();
                Some(s.to_string())
            })
            .collect();
        if !sigs.is_empty() {
            return sigs;
        }
        let mut sigs = vec![];
        let mut s = result.as_str();
        while let Some(i) = s.find('`') {
            s = &s[i + 1..];
            let i = some_or!(s.find('`'), break);
            let sig = &s[..i].trim();
            let sig = sig.strip_prefix("unsafe ").unwrap_or(sig).trim();
            let sig = sig.strip_suffix(';').unwrap_or(sig).trim();
            if sig.starts_with("fn ") {
                sigs.push(sig.to_string());
            }
            s = &s[i + 1..];
        }
        sigs
    }

    async fn translate_function(
        &self,
        code: &str,
        signature: Option<&str>,
        deps: &[String],
    ) -> Option<String> {
        let m1 = system("You are a helpful assistant that translates C to Rust.");
        let deps = if deps.is_empty() {
            "".to_string()
        } else {
            format!(
                "The following definition{} been translated from C to Rust already:
```
{}
```
",
                if deps.len() == 1 { " has" } else { "s have" },
                deps.join("\n")
            )
        };
        let sig = if let Some(signature) = signature {
            format!(
                "Your answer must start with:
```
{} {{
```
",
                signature
            )
        } else {
            "".to_string()
        };
        let prompt = format!(
            "{}Translate the following C function to Rust using Rust idioms without any explanation:
```
{}
```
{}Try to avoid unsafe code. Do not add `use` statements. Use full paths instead.",
            deps, code, sig
        );
        let m2 = user(&prompt);
        let msgs = vec![m1, m2];
        let result = self.send_request(msgs, None).await;
        let result = extract_code(&result, &["fn ", "unsafe fn "]).or_else(|| {
            if result.starts_with("fn ") | result.starts_with("unsafe fn ") {
                Some(result)
            } else {
                None
            }
        })?;
        let mut r = String::new();
        for line in result.split('\n') {
            match line.strip_prefix("unsafe fn ") {
                Some(line) => {
                    r.push_str("fn ");
                    r.push_str(line);
                    r.push('\n');
                }
                None => {
                    r.push_str(line);
                    r.push('\n');
                }
            }
        }
        Some(r)
    }

    async fn fix(&self, code: &str, error: &str) -> Option<String> {
        let m1 = system("You are a helpful assistant.");
        let instruction = "Explain the error first and then write the code of the fixed function.";
        let prompt = format!(
            "The following Rust code has a compilation error:
```
{}
```
The error message is:
```
{}
```
{}
",
            code, error, instruction
        );
        let m2 = user(&prompt);
        let msgs = vec![m1, m2];
        let result = self.send_request(msgs, None).await;
        extract_code(
            &result,
            &[
                "type ", "struct ", "union ", "enum ", "const ", "static ", "fn ",
            ],
        )
    }

    async fn compare(&self, code1: &str, code2: &str) -> std::cmp::Ordering {
        if tokens_in_str(code1) + tokens_in_str(code2) > 3820 {
            return std::cmp::Ordering::Equal;
        }
        let m1 = system("You are a helpful assistant.");
        let m2 = user(
            "Consider two following Rust functions:
Implementation 1
```
fn div(n: u32, d: u32) -> i32 {
    if d == 0 {
        return -1;
    }
    (n / d) as i32
}
```
Implementation 2
```
fn div(n: u32, d: u32) -> Option<u32> {
    if d == 0 {
        return None;
    }
    Some(n / d)
}
```
Which one is more Rust-idiomatic? Compare them and choose one.
Your answer format is:

Comparison:
[comparison]
Choice: Implementation [n]",
        );
        let m3 = assistant("Comparison:
Both handle the case where the denominator is zero, but they do it differently. Implementation 1 returns -1, which is not a valid result for the division operation, while implementation 2 returns an Option type, which is a more idiomatic way of handling errors in Rust. Additionally, implementation 2 returns an unsigned integer instead of a signed integer, which is more appropriate for the result of a division operation.
Choice: Implementation 2");
        let prompt = format!(
            "Consider two following Rust functions:
Implementation 1
```
{}
```
Implementation 2
```
{}
```
Which one is more Rust-idiomatic? Compare them and choose one.
Your answer format is:

Comparison:
[comparison]
Choice: Implementation [n]",
            code1, code2
        );
        let m4 = user(&prompt);
        let msgs = vec![m1, m2, m3, m4];
        let result = self.send_request(msgs, None).await;
        let s = "Choice: Implementation ";
        let i = some_or!(result.find(s), return std::cmp::Ordering::Equal);
        let c = some_or!(
            result[i + s.len()..]
                .chars()
                .find(|&c| c == '1' || c == '2'),
            return std::cmp::Ordering::Equal
        );
        match c {
            '1' => std::cmp::Ordering::Greater,
            '2' => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Equal,
        }
    }
}

fn make_deps(deps: &[String]) -> String {
    if deps.is_empty() {
        "".to_string()
    } else {
        format!(
            "The following definition{} been translated from C to Rust already:
```
{}
```
",
            if deps.len() == 1 { " has" } else { "s have" },
            deps.join("\n")
        )
    }
}

fn signature_prompt(code: &str, new_name: &str, deps: &[String], n: usize) -> String {
    let sigs: String = (1..=n).map(|i| format!("{}. `signature`\n", i)).collect();
    format!(
        "{}Consider the following C function:
```
{}
```
If this function was written in Rust with Rust idioms, what would be its signature?
First, explain the function. Then, give {} Rust-idiomatic candidate signature{}.
Do not add additional parameters to the signatures.
The answer format is:

Explanation:
[explanation]
Signatures:
{}
Each signature must look like `fn {5}(...);` or `fn {5}(...) -> ...;`.",
        make_deps(deps),
        code,
        n,
        if n == 1 { "" } else { "s" },
        sigs,
        new_name
    )
}

fn extract_name(result: String) -> String {
    let i = some_or!(result.find('`'), return result);
    let result = &result[i + 1..];
    let i = result.find('`').unwrap();
    result[..i].to_string()
}

fn extract_code(mut result: &str, prefixes: &[&str]) -> Option<String> {
    let pat1 = "```rust\n";
    let pat2 = "```\n";
    let pat3 = "\n```";

    let mut results: Vec<_> = vec![];
    loop {
        let i1 = result.find(pat1).map(|i| i + pat1.len());
        let i2 = result.find(pat2).map(|i| i + pat2.len());
        let i = match (i1, i2) {
            (Some(i1), Some(i2)) => std::cmp::min(i1, i2),
            (i1, i2) => some_or!(i1.or(i2), break),
        };
        result = &result[i..];
        let i = some_or!(result.find(pat3), break);
        results.push(&result[..i]);
        result = &result[i + pat3.len()..];
    }

    results.retain(|s| {
        s.lines()
            .any(|line| prefixes.iter().any(|prefix| line.starts_with(prefix)))
    });

    results
        .into_iter()
        .max_by_key(|s| s.len())
        .map(|s| s.to_string())
}

fn role_to_str(role: &Role) -> &'static str {
    match role {
//         Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

#[allow(unused)]
fn num_tokens(msgs: &[Message]) -> usize {
    msgs.iter()
        .map(|msg| {
            let role_token_count = tokens_in_str(role_to_str(&msg.role)); // 计算角色的 token 数量

            let content_blocks: Vec<ContentBlock> = msg.content.clone();
            let content: String = content_blocks.iter()
                .filter_map(|block| {
                    if let ContentBlock::Text { text } = block {
                        Some(text.clone()) // 提取并克隆文本
                    } else {
                        None // 忽略非 Text 类型的块
                    }
                })
                .collect::<Vec<String>>()
                .join(" "); // 用空格连接所有文本
            // 提取字符串并计算内容的 token 数量
            let content_token_count = tokens_in_str(&content);

            4 + role_token_count + content_token_count
            })
        .sum::<usize>()
        + 3

}

fn system(s: &str) -> Message {
    Message {
        role: Role::User,
        content: vec![ContentBlock::Text { text: s.to_string() }]
//         name: None,
    }
}

fn user(s: &str) -> Message {
    Message {
        role: Role::User,
        content: vec![ContentBlock::Text { text: s.to_string() }]
//         name: None,
    }
}

fn assistant(s: &str) -> Message {
    Message {
        role: Role::User,
        content: vec![ContentBlock::Text { text: s.to_string() }]
//         name: None,
    }
}
// 其他实现部分保持不变...
