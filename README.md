# claude-plugin-sicompass

*Claude Code, in Sicompass.*

This plugin is part of [Sicompass](https://github.com/friendlyflow/sicompass), a
keyboard-first, accessibility-first way to use your entire computer.

Claude starts on a folder listing. Walk to a project and press : for its
sessions, the ones run in that folder and below it. Right on one resumes it, and
new session starts a fresh one. The conversation is a list of rows: your
prompts, Claude's answers, every tool call and its result. i on the last row
types the next prompt, and a second : lists the project's skills and Claude
Code's slash commands to insert.

It drives the `claude` command line tool you already have, with the login that
tool already has, so there is no key to paste here. Install Claude Code first.
Sicompass finds `claude` on your `PATH` or in `~/.local/bin`.

Claude asks for your whole disk, to pick a project and read Claude Code's own
folder, and to run `claude`. The Store shows that before you install it, and
installing it is your approval.

## Install

In Sicompass, open store, then programs, and press Enter on install next to
claude. The Store checks the release's signature before installing it, and
keeps it up to date.

## Building from source

```bash
nix develop          # the toolchain, with the wasm32-wasip2 target
cargo test           # natively
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/claude_plugin.wasm plugin.wasm
```

`./scripts/release-plugin.sh --dry-run` does the build, checks the component
against `plugin.json`, and signs and verifies it with a throwaway key, the way
a release is made.

## Related repositories

- [sicompass](https://github.com/friendlyflow/sicompass), the application
- [sicompass-plugin-sdk](https://github.com/friendlyflow/sicompass-plugin-sdk),
  the SDK, the WASM plugin kit and the cloud backup library

## Community

Join the conversation on
[Discord](https://discord.com/channels/1464152138753249313/1464152139231137894).

## License

#### Open source license

If you are creating an open source application under a license compatible with
the GNU GPL license v3, you may use this project under the terms of the GPLv3.
See [LICENSE](LICENSE).

## Contributing

Contributions are welcome. Whether it is code, documentation, or feedback, your
input helps make computing more accessible for everyone.
