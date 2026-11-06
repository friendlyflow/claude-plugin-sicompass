# Changelog

## 0.3.3

- The session list and the skills list appear at once. They used to be read again every time the plugin started, which took several seconds after an update: the transcripts for the session list, and the whole Claude Code program for its slash commands. The plugin now keeps what it read in its own folder, and reads the commands in the background when a tab opens.
- Needs sicompass 0.2.4.

## 0.3.2

- Tool results, folders and session ids are shown exactly as written, never read as markup, and a prompt is sent with its own bytes.
- Needs sicompass 0.2.4.
- Intel macs are not built any more.

## 0.3.1

- The new session button opens an empty session. You type the first prompt on
  the session's own prompt row, where Ctrl+: lists the skills, and Claude starts
  when you send it.

## 0.3.0

Claude is a program of its own now, instead of a sandboxed WebAssembly component.
Sicompass starts it and talks to it, one per tab, and it runs with your rights, so
its entry in the Store says what it does before you install it, and installing it is
your approval.

- It runs your `claude` program, found on your `PATH`, in `~/.local/bin`, or (macOS) in Applications. The tab switcher shows the session's program.
- One build for each of Linux (x86_64 and arm64, static), macOS (Apple Silicon and
  Intel) and Windows.
- Needs a Sicompass that runs plugin programs. An older Sicompass keeps the 0.2
  version it has.
