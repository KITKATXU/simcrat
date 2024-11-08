# simcrat-gpt4o

[GitHub - kaist-plrg/simcrat](https://github.com/kaist-plrg/simcrat): *Type-Migrating C-to-Rust Translation using a Large Language Model*

## Update

Updated to support Claude 3.5 version: `./run-claude35.sh`

- **gzip-1.12**: 
  - 6383 LOC
  - 41 Types
  - 151 Variables
  - 20 Protos
  - 220 Functions
  - 1144 Calls

#### Method

1. Let LLM generate a specified number of candidate function signatures.
2. Provide the function signatures to the caller function for translation.
3. Let LLM compare the translation quality of different function definitions.

#### Experimental Attempts

| Model      | Request Tokens | Response Tokens | Response Time (s) | LOC  | Func | Errors | Warnings |
|------------|----------------|------------------|--------------------|------|------|--------|----------|
| Gpt-35     |                |                  |                    | 7047 | 248  | 884    | 86       |
| Gpt-4o     | 541042         | 916480           | 3405.4412          | 7783 | 247  | 487    | 34       |
| Claude-35  | 498517         | 931402           | 795.5489           | 6906 | 246  | 207    | 12       |

    
#### Translation Results

**Strengths**
- Translated types, variables, and functions one by one, with the output also following this order.
- Very organized and appearing to have no omissions in any of the aforementioned definitions.

**Weaknesses**
- Empty main function.
- Unsafe code blocks.
- Increased Number of Functions:
Reason: A significant portion of these new functions are defined within the bodies of existing functions. Many are related to error handling; for instance, the goto statements supported in C are translated into new function declarations in Rust, such as goto_fail, goto_name_too_long, etc.
- Calls to Non-Existent Functions:
Despite the fact that all callee function declarations were passed in according to the call graph, the translated functions still invoke non-existent functions that have not been implemented.
