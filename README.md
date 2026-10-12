# nu_plugin_ssh_agent

A small Nushell plugin that starts `ssh-agent` when needed and reuses it across
shell sessions. It stores the agent's environment in `~/.ssh/env` with private
permissions and validates the saved process before reuse.

## Build and register

Requires Rust/Cargo, Nushell, and OpenSSH's `ssh-agent`.

```sh
cargo build --release
plugin add target/release/nu_plugin_ssh_agent
plugin use nu_plugin_ssh_agent
```

Add this to `~/.config/nushell/config.nu` to load the environment for each
interactive shell:

```nu
load-env (ssh-agent env)
```

The command returns a record containing `SSH_AUTH_SOCK` and `SSH_AGENT_PID`.
It does not automatically unlock or add private keys. To add keys on SSH use,
configure `~/.ssh/config` as desired, for example:

```sshconfig
Host *
    AddKeysToAgent yes
```

## Development

```sh
cargo test
cargo fmt --check
cargo clippy -- -D warnings
```

MIT licensed.
