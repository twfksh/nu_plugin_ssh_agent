use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use nu_plugin::{
    serve_plugin, EvaluatedCall, MsgPackSerializer, Plugin, PluginCommand, SimplePluginCommand,
};
use nu_protocol::{LabeledError, Signature, Span, Value};

struct SshAgentPlugin;
struct Env;

impl Plugin for SshAgentPlugin {
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }

    fn commands(&self) -> Vec<Box<dyn PluginCommand<Plugin = Self>>> {
        vec![Box::new(Env)]
    }
}

impl SimplePluginCommand for Env {
    type Plugin = SshAgentPlugin;

    fn name(&self) -> &str {
        "ssh-agent env"
    }

    fn description(&self) -> &str {
        "Return environment variables for a running ssh-agent, starting one if needed"
    }

    fn signature(&self) -> Signature {
        Signature::build("ssh-agent env")
    }

    fn run(
        &self,
        _plugin: &SshAgentPlugin,
        _engine: &nu_plugin::EngineInterface,
        call: &EvaluatedCall,
        _input: &Value,
    ) -> Result<Value, LabeledError> {
        let env_file = env_file_path().map_err(|e| error(e, call.head))?;
        let agent = match read_agent_env(&env_file) {
            Ok(Some(agent)) if agent_is_running(&agent) => agent,
            _ => start_agent(&env_file).map_err(|e| error(e, call.head))?,
        };

        let span = call.head;
        Ok(Value::record(
            nu_protocol::record! {
                "SSH_AUTH_SOCK" => Value::string(agent.socket, span),
                "SSH_AGENT_PID" => Value::string(agent.pid, span),
            },
            span,
        ))
    }
}

struct AgentEnv {
    socket: String,
    pid: String,
}

fn env_file_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    let ssh_dir = PathBuf::from(home).join(".ssh");
    fs::create_dir_all(&ssh_dir).map_err(|e| format!("cannot create ~/.ssh: {e}"))?;
    Ok(ssh_dir.join("env"))
}

fn read_agent_env(path: &Path) -> Result<Option<AgentEnv>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    Ok(parse_agent_env(&text))
}

fn parse_agent_env(text: &str) -> Option<AgentEnv> {
    let mut socket = None;
    let mut pid = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("SSH_AUTH_SOCK=") {
            socket = Some(value.split(';').next()?.trim_matches('\'').to_owned());
        } else if let Some(value) = line.strip_prefix("SSH_AGENT_PID=") {
            pid = Some(value.split(';').next()?.trim_matches('\'').to_owned());
        }
    }
    let socket = socket?;
    let pid = pid?;
    if socket.is_empty() || !pid.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(AgentEnv { socket, pid })
}

fn agent_is_running(agent: &AgentEnv) -> bool {
    let Ok(pid) = agent.pid.parse::<u32>() else {
        return false;
    };
    let Ok(output) = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
    else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .trim()
            .rsplit('/')
            .next()
            == Some("ssh-agent")
        && Path::new(&agent.socket).exists()
}

fn start_agent(env_file: &Path) -> Result<AgentEnv, String> {
    let output = Command::new("ssh-agent")
        .arg("-s")
        .output()
        .map_err(|e| format!("could not run ssh-agent: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ssh-agent failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|e| format!("ssh-agent returned invalid output: {e}"))?;
    let agent = parse_agent_env(&text).ok_or("could not parse ssh-agent environment")?;
    write_private_env(env_file, &text)?;
    Ok(agent)
}

fn write_private_env(path: &Path, contents: &str) -> Result<(), String> {
    let temp = path.with_extension(format!("env.{}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        Ok::<_, std::io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(|e| format!("cannot securely write {}: {e}", path.display()))
}

fn error(message: impl Into<String>, span: Span) -> LabeledError {
    LabeledError::new("ssh-agent error").with_label(message.into(), span)
}

fn main() {
    serve_plugin(&SshAgentPlugin, MsgPackSerializer);
}

#[cfg(test)]
mod tests {
    use super::parse_agent_env;

    #[test]
    fn parses_ssh_agent_shell_output() {
        let env = parse_agent_env(
            "SSH_AUTH_SOCK=/tmp/ssh-abc/agent.123; export SSH_AUTH_SOCK;\nSSH_AGENT_PID=123; export SSH_AGENT_PID;\n",
        )
        .unwrap();
        assert_eq!(env.socket, "/tmp/ssh-abc/agent.123");
        assert_eq!(env.pid, "123");
    }

    #[test]
    fn rejects_incomplete_or_invalid_output() {
        assert!(parse_agent_env("SSH_AGENT_PID=12;").is_none());
        assert!(parse_agent_env("SSH_AUTH_SOCK=/tmp/a;\nSSH_AGENT_PID=nope;").is_none());
    }
}
