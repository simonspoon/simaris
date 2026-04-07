use std::process::Command;

struct TestEnv {
    dir: std::path::PathBuf,
}

impl TestEnv {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("simaris-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        TestEnv { dir }
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        let bin = env!("CARGO_BIN_EXE_simaris");
        Command::new(bin)
            .args(args)
            .env("SIMARIS_HOME", &self.dir)
            .output()
            .expect("Failed to execute simaris")
    }

    fn run_ok(&self, args: &[&str]) -> String {
        let output = self.run(args);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        assert!(
            output.status.success(),
            "Command failed: simaris {}\nstdout: {stdout}\nstderr: {stderr}",
            args.join(" ")
        );
        stdout
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn test_add_command() {
    let env = TestEnv::new("add");
    let out = env.run_ok(&["add", "hello world", "--type", "fact"]);
    assert!(out.contains("Added unit 1"), "got: {out}");
}

#[test]
fn test_show_command() {
    let env = TestEnv::new("show");
    env.run_ok(&[
        "add",
        "some knowledge",
        "--type",
        "principle",
        "--source",
        "test",
    ]);
    let out = env.run_ok(&["show", "1"]);
    assert!(out.contains("some knowledge"), "got: {out}");
    assert!(out.contains("principle"), "got: {out}");
    assert!(out.contains("test"), "got: {out}");
}

#[test]
fn test_link_command() {
    let env = TestEnv::new("link");
    env.run_ok(&["add", "unit a", "--type", "fact"]);
    env.run_ok(&["add", "unit b", "--type", "idea"]);
    let out = env.run_ok(&["link", "1", "2", "--rel", "related-to"]);
    assert!(out.contains("Linked 1 -> 2"), "got: {out}");
}

#[test]
fn test_show_with_links() {
    let env = TestEnv::new("showlinks");
    env.run_ok(&["add", "unit a", "--type", "fact"]);
    env.run_ok(&["add", "unit b", "--type", "idea"]);
    env.run_ok(&["link", "1", "2", "--rel", "depends-on"]);

    let out = env.run_ok(&["show", "1"]);
    assert!(out.contains("-> 2 (depends_on)"), "got: {out}");

    let out = env.run_ok(&["show", "2"]);
    assert!(out.contains("<- 1 (depends_on)"), "got: {out}");
}

#[test]
fn test_env_override() {
    let env = TestEnv::new("envoverride");
    env.run_ok(&["add", "env test", "--type", "lesson"]);
    assert!(env.dir.join("sanctuary.db").exists());
}

#[test]
fn test_json_output() {
    let env = TestEnv::new("json");
    let out = env.run_ok(&["--json", "add", "json test", "--type", "fact"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(parsed["id"], 1);

    let out = env.run_ok(&["--json", "show", "1"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(parsed["unit"]["content"], "json test");
    assert_eq!(parsed["unit"]["type"], "fact");
}

#[test]
fn test_drop_command() {
    let env = TestEnv::new("drop");
    let out = env.run_ok(&["drop", "raw idea about caching"]);
    assert!(out.contains("Dropped item 1"), "got: {out}");

    let out = env.run_ok(&["inbox"]);
    assert!(out.contains("raw idea about caching"), "got: {out}");
}

#[test]
fn test_drop_command_custom_source() {
    let env = TestEnv::new("dropsource");
    env.run_ok(&["drop", "phone idea", "--source", "phone"]);
    let out = env.run_ok(&["inbox"]);
    assert!(out.contains("(phone)"), "got: {out}");
}

#[test]
fn test_inbox_empty() {
    let env = TestEnv::new("inboxempty");
    let out = env.run_ok(&["inbox"]);
    assert!(out.contains("Inbox is empty."), "got: {out}");
}

#[test]
fn test_drop_empty_content_rejected() {
    let env = TestEnv::new("dropempty");
    let output = env.run(&["drop", ""]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Content cannot be empty"),
        "got stderr: {stderr}"
    );
}

#[test]
fn test_inbox_json_output() {
    let env = TestEnv::new("inboxjson");
    env.run_ok(&["drop", "first thought"]);
    env.run_ok(&["drop", "second thought", "--source", "api"]);

    let out = env.run_ok(&["--json", "inbox"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON array");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0]["content"], "first thought");
    assert_eq!(parsed[0]["source"], "cli");
    assert_eq!(parsed[1]["content"], "second thought");
    assert_eq!(parsed[1]["source"], "api");
    assert!(parsed[0]["id"].is_number());
    assert!(parsed[0]["created"].is_string());
}

#[test]
fn test_promote_command() {
    let env = TestEnv::new("promote");
    env.run_ok(&["drop", "caching matters for perf"]);

    let out = env.run_ok(&["promote", "1", "--type", "fact"]);
    assert!(out.contains("Added unit 1"), "got: {out}");

    let out = env.run_ok(&["show", "1"]);
    assert!(out.contains("caching matters for perf"), "got: {out}");
    assert!(out.contains("fact"), "got: {out}");

    let out = env.run_ok(&["inbox"]);
    assert!(out.contains("Inbox is empty."), "got: {out}");
}

#[test]
fn test_promote_nonexistent_id() {
    let env = TestEnv::new("promotebad");
    let output = env.run(&["promote", "999", "--type", "fact"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Inbox item 999 not found"),
        "got stderr: {stderr}"
    );
}
