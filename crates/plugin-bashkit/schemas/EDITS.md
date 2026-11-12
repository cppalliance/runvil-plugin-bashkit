# Definition edits

`shell.json` is Cursor's definition of its `Shell` tool, snapshot on 2026-11-12 from the tool list of a Cursor agent session. Everything in the file is Cursor's text except the edits below, each made because the line describes behavior this Pack does not have.

## shell.json

- `with optional foreground timeout` became `with an optional time limit`, because the shell has no background: a command that passes its limit is stopped.
- Removed `IMPORTANT: Use this tool for terminal operations like git, npm, docker, etc. DO NOT use it for file operations (reading, writing, editing, searching, finding files, sleeping) - use the specialized tools for this instead.` The shell runs no external program, so a model told otherwise would call `git` and get an error. In its place a paragraph says what the shell is: a bash interpreter inside the run, with every command built in and no network.
- Removed the bullet that names `Grep`, `Glob`, `Read`, and `rg`, because another Pack offers those tools, and a run may hold this Pack without it.
- Removed the bullet about `head`, `tail`, `sed -n`, and the terminal file, because no output is saved to a file.
- Removed `Run independent commands as parallel Shell calls.`, because the calls of a run share one session and run one at a time. `Chain dependent commands with &&.` is kept.
- Removed the bullet about the `gh` command, which the shell does not have.
- Added four bullets that state what the shell does: the session lasts for the run; the shell has files of its own and does not see the run's files; output past `{output_cap}` characters is cut in the middle; and calls run one at a time. The placeholder is filled when the definition is built, from the constant the code enforces, so the text and the limit cannot drift apart.
- `block_until_ms` is an `integer` of at least 1, where Cursor's is a `number`, because a count of milliseconds is whole and 0 has no meaning here. Cursor's text is `How long to block and wait for the command to complete before moving it to background (in milliseconds). Defaults to 30000ms (30 seconds). Set to 0 to immediately run the command in the background. The timer includes the shell startup time.` This one says the command is stopped at the limit, that the default is `{timeout_ms}`ms, and that a larger value is lowered to that. The placeholder is filled from the constant the code enforces.
- Removed the parameters `notify_on_output`, `request_smart_mode_approval`, and `smart_mode_block_reason`, which belong to Cursor's output monitoring and its approval flow.
- `command`, `working_directory`, and `description` are verbatim. `description` is accepted and not used.
