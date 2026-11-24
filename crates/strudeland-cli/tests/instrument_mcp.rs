//! Exercise the shipped stdio entry point, including live calls across the control socket.
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

use serde_json::{Value, json};
use strudeland_project::Project;

struct Mcp {
    child: Child,
    lines: Receiver<String>,
    id: u32,
}

impl Mcp {
    fn start(command: &mut Command) -> Self {
        let mut child = command
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if tx.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            lines,
            id: 0,
        };
        let init = client.request(
            "initialize",
            json!({
                "protocolVersion":"2025-11-25", "capabilities":{},
                "clientInfo":{"name":"instrument-test","version":"1"}
            }),
        );
        assert_eq!(init["protocolVersion"], "2025-11-25");
        assert_eq!(init["serverInfo"]["name"], "strudeland");
        assert!(init["capabilities"]["tools"].is_object());
        client.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        client
    }

    fn send(&mut self, message: Value) {
        writeln!(self.child.stdin.as_mut().unwrap(), "{message}").unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        self.send(json!({"jsonrpc":"2.0", "id":self.id, "method":method, "params":params}));
        let line = self
            .lines
            .recv_timeout(Duration::from_secs(15))
            .expect("MCP response");
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], self.id, "{response}");
        assert_eq!(response["jsonrpc"], "2.0");
        assert!(response.get("error").is_none(), "{response}");
        response["result"].clone()
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": arguments}))
    }

    fn finish(&mut self) {
        drop(self.child.stdin.take());
        assert_eq!(
            self.lines.recv_timeout(Duration::from_secs(15)),
            Err(RecvTimeoutError::Disconnected)
        );
        assert!(self.child.wait().unwrap().success());
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn cli(project: &Project) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_strudeland"));
    command.current_dir(project.root());
    command
}

#[test]
fn mcp_discovers_five_typed_tools_and_lists_the_enclosing_project() {
    let temp = tempfile::tempdir().unwrap();
    let project = Project::create(&temp.path().join("demo"), "test").unwrap();
    let mut client = Mcp::start(&mut cli(&project));
    let discovery = client.request("tools/list", json!({}));
    let tools = discovery["tools"].as_array().unwrap();
    let mut names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "instrument_apply",
            "instrument_create",
            "instrument_list",
            "instrument_pin",
            "instrument_update"
        ]
    );
    let apply = tools
        .iter()
        .find(|t| t["name"] == "instrument_apply")
        .unwrap();
    let properties = &apply["inputSchema"]["properties"];
    assert_eq!(properties["from"]["type"], "integer");
    assert_eq!(properties["expect"]["type"], "string");
    let args = json!({"source":"project"});
    let listing = client.call("instrument_list", args)["structuredContent"].clone();
    assert_eq!(
        listing,
        json!({"directory": project.root().join("instruments"), "entries":[]})
    );
    let invalid = temp.path().join("missing");
    let error = client.call(
        "instrument_list",
        json!({"source":"project","project":invalid}),
    );
    let output = cli(&project)
        .args(["instrument", "list", "--project"])
        .arg(invalid)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(error["isError"], true);
    assert_eq!(
        error["content"][0]["text"],
        String::from_utf8(output.stderr).unwrap().trim_end()
    );
    client.finish();
}

#[test]
#[cfg(unix)]
fn mcp_creates_pins_updates_and_applies_through_the_control_socket() {
    use std::fs;
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    let temp = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in("/tmp")
        .unwrap();
    let project = Project::create(&temp.path().join("demo"), "test").unwrap();
    let socket = temp.path().join("s");
    let command = || {
        let mut cmd = cli(&project);
        cmd.env("HOME", temp.path())
            .env("XDG_DATA_HOME", temp.path())
            .arg("--socket")
            .arg(&socket);
        cmd
    };
    let mut client = Mcp::start(&mut command());
    let list = client.call("instrument_list", json!({"source":"library"}));
    let library = Path::new(list["structuredContent"]["directory"].as_str().unwrap());
    assert!(library.starts_with(temp.path().canonicalize().unwrap()));
    let mut definition = json!({"schema":1, "parameters":{}, "voices":[
        {"name":"lead","chain":[{"control":"s","value":"sine"}]}
    ]});
    let input = temp.path().join("preset.json");
    fs::write(&input, definition.to_string()).unwrap();
    let human = command()
        .args(["instrument", "create", "bass", "--file"])
        .arg(input)
        .output()
        .unwrap();
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    let listing = client.call("instrument_list", json!({"source":"library"}));
    let bass = &listing["structuredContent"]["entries"][0];
    assert_eq!(bass["id"], "bass");
    assert_eq!(
        String::from_utf8(human.stdout).unwrap(),
        format!("bass  {}\n", bass["fingerprint"].as_str().unwrap())
    );

    let create = json!({"id":"lead","definition":definition});
    let entry = client.call("instrument_create", create.clone())["structuredContent"].clone();
    assert_eq!(entry["id"], "lead");
    assert_eq!(client.call("instrument_create", create)["isError"], true);
    let pin = client.call("instrument_pin", json!({"id":"lead"}));
    assert_eq!(pin["structuredContent"], entry);
    definition["voices"][0]["chain"][0]["value"] = json!("sawtooth");
    fs::write(library.join("lead.json"), definition.to_string()).unwrap();
    let update = json!({"id":"lead","expect":entry["fingerprint"]});
    let updated = client.call("instrument_update", update.clone())["structuredContent"].clone();
    assert_eq!(updated["id"], "lead");
    assert_ne!(updated["fingerprint"], entry["fingerprint"]);
    assert_eq!(client.call("instrument_update", update)["isError"], true);

    let listener = UnixListener::bind(socket.as_path()).unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        let result = json!({"song":"songs/main.js","from":8,"to":48});
        writeln!(
            writer,
            "{}",
            json!({"jsonrpc":"2.0","id":request["id"],"result":result})
        )
        .unwrap();
        request
    });
    let source = "// 🎵\r\n note(\"e4\")\n";
    let input = json!({"id":"lead","song":"songs/main.js","from":8,"to":18,"expect":source});
    let applied = client.call("instrument_apply", input.clone());
    assert_eq!(
        applied["structuredContent"],
        json!({"song":"songs/main.js","from":8,"to":48})
    );
    let request = server.join().unwrap();
    assert_eq!(request["method"], "instrument.apply");
    let mut expected = input;
    expected["root"] = json!(project.root());
    assert_eq!(request["params"], expected);
    client.finish();
}
