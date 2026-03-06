# Rules


You should ALWAYS reference the @problems (diagnostics) for the compilation errors and not constantly run "cargo check" or "cargo build"


## Commands
Use only Windows commands, not linux commands

Do NOT try to parse output with "head" like this "cargo build 2>&1 | head -100"

Do NOT try to parse cargo build with select-string like this "cargo build 2>&1 | Select-Object -First 100"

Do NOT run "cargo build --release" only run "cargo build"

Do NOT run "cargo run" ask the user for output of the compiled software

Do NOT run "cargo clean" without asking the user for permission.



### Web Search
Use your web search tools if you're stuck and the source code review is not helping


## Task Difficulty

If something is too difficult, break it down into as many steps as needed steps. NEVER give up on a task due to difficulty.

## Lessons Learned in pitfalls.md

When you fix an issue AND the user confirms it's resolved, note the interaction and details in pitfalls.md so that future work can benefit from the lessons learned. Do not edit pitfalls.md until the user confirms the issue is fixed. Your notes should be short and concise.


## Issue Tracking

When working on an issue, note what you attempted in a .md file dedicated to the issue. This should be reviewed everytime context is compressed to prevent repeatedly trying the same thing. The file should be cleaned up when the issue is confirmed as fixed.


## Finishing

Before ending a task, confirm that "cargo build" is successful.

All compilation errors (not warnings) must be resolved.