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

#[test]
fn test_list_command() {
    let env = TestEnv::new("list");
    env.run_ok(&["add", "fact one", "--type", "fact"]);
    env.run_ok(&["add", "procedure one", "--type", "procedure"]);
    env.run_ok(&["add", "idea one", "--type", "idea"]);
    let out = env.run_ok(&["list"]);
    assert!(out.contains("fact one"), "got: {out}");
    assert!(out.contains("procedure one"), "got: {out}");
    assert!(out.contains("idea one"), "got: {out}");
}

#[test]
fn test_list_filter() {
    let env = TestEnv::new("listfilter");
    env.run_ok(&["add", "fact one", "--type", "fact"]);
    env.run_ok(&["add", "fact two", "--type", "fact"]);
    env.run_ok(&["add", "idea one", "--type", "idea"]);
    let out = env.run_ok(&["list", "--type", "fact"]);
    assert!(out.contains("fact one"), "got: {out}");
    assert!(out.contains("fact two"), "got: {out}");
    assert!(!out.contains("idea one"), "got: {out}");
}

#[test]
fn test_search_command() {
    let env = TestEnv::new("search");
    env.run_ok(&["add", "caching improves performance", "--type", "fact"]);
    env.run_ok(&["add", "deploy with cargo install", "--type", "procedure"]);
    let out = env.run_ok(&["search", "caching"]);
    assert!(out.contains("caching improves performance"), "got: {out}");
    assert!(!out.contains("deploy"), "got: {out}");
}

#[test]
fn test_search_empty_result() {
    let env = TestEnv::new("searchempty");
    env.run_ok(&["add", "some content", "--type", "fact"]);
    let output = env.run(&["search", "nonexistent"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No units found."), "got: {stdout}");
}

#[test]
fn test_list_json_output() {
    let env = TestEnv::new("listjson");
    env.run_ok(&["add", "fact one", "--type", "fact", "--source", "test"]);
    env.run_ok(&["add", "idea one", "--type", "idea", "--source", "test"]);

    let out = env.run_ok(&["--json", "list"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON array");
    assert_eq!(parsed.len(), 2);
    assert!(parsed.iter().any(|u| u["content"] == "fact one"));
    assert!(parsed.iter().any(|u| u["content"] == "idea one"));
    assert!(parsed[0]["id"].is_number());
    assert!(parsed[0]["type"].is_string());
}

#[test]
fn test_backup_command() {
    let env = TestEnv::new("backup");
    env.run_ok(&["add", "important knowledge", "--type", "fact"]);
    let out = env.run_ok(&["backup"]);
    assert!(out.contains("Backup created:"), "got: {out}");
    let backups_dir = env.dir.join("backups");
    assert!(backups_dir.exists(), "backups directory should exist");
    let entries: Vec<_> = std::fs::read_dir(&backups_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1, "should have exactly one backup");
    let name = entries[0].file_name().to_str().unwrap().to_string();
    assert!(
        name.starts_with("sanctuary-") && name.ends_with(".db"),
        "backup name should match pattern: {name}"
    );
}

#[test]
fn test_restore_command() {
    let env = TestEnv::new("restore");
    // Seed original data
    env.run_ok(&["add", "original knowledge", "--type", "fact"]);
    // Create backup
    env.run_ok(&["backup"]);
    // Add more data after backup
    env.run_ok(&["add", "extra stuff", "--type", "lesson"]);
    // Verify both items exist
    let out = env.run_ok(&["--json", "list"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(parsed.len(), 2, "should have 2 units before restore");

    // Find the backup filename
    let backups_dir = env.dir.join("backups");
    let backup_name = std::fs::read_dir(&backups_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .next()
        .unwrap()
        .file_name()
        .to_str()
        .unwrap()
        .to_string();

    // Restore
    let out = env.run_ok(&["restore", &backup_name]);
    assert!(out.contains("Restored from:"), "got: {out}");

    // Verify only original data remains
    let out = env.run_ok(&["--json", "list"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(
        parsed.len(),
        1,
        "should have 1 unit after restore, got: {out}"
    );
    assert_eq!(parsed[0]["content"], "original knowledge");
}

#[test]
fn test_backup_prune() {
    let env = TestEnv::new("prune");
    let backups_dir = env.dir.join("backups");
    std::fs::create_dir_all(&backups_dir).unwrap();
    // Create 12 backup files manually to avoid timing issues
    for i in 0..12 {
        let name = format!("sanctuary-20260101-{:06}.db", i);
        std::fs::write(backups_dir.join(&name), "fake backup").unwrap();
    }
    // Seed data and run a real backup (which triggers pruning)
    env.run_ok(&["add", "prune test", "--type", "fact"]);
    env.run_ok(&["backup"]);
    // Count remaining backups
    let remaining: Vec<_> = std::fs::read_dir(&backups_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_str().unwrap().to_string();
            name.starts_with("sanctuary-") && name.ends_with(".db")
        })
        .collect();
    assert!(
        remaining.len() <= 10,
        "should have at most 10 backups after pruning, got: {}",
        remaining.len()
    );
}

#[test]
fn test_restore_list() {
    let env = TestEnv::new("restorelist");
    let backups_dir = env.dir.join("backups");
    std::fs::create_dir_all(&backups_dir).unwrap();
    // Create two backup files
    std::fs::write(
        backups_dir.join("sanctuary-20260101-000000.db"),
        "fake backup 1",
    )
    .unwrap();
    std::fs::write(
        backups_dir.join("sanctuary-20260201-000000.db"),
        "fake backup 2",
    )
    .unwrap();
    // List backups (restore with no args)
    let out = env.run_ok(&["restore"]);
    assert!(
        out.contains("sanctuary-20260101-000000.db"),
        "should list first backup: {out}"
    );
    assert!(
        out.contains("sanctuary-20260201-000000.db"),
        "should list second backup: {out}"
    );
}

#[test]
fn test_digest_empty_inbox() {
    let env = TestEnv::new("digestempty");
    let out = env.run_ok(&["digest"]);
    assert!(
        out.contains("Nothing to digest") || out.contains("empty"),
        "got: {out}"
    );
}

#[test]
fn test_env_dev_isolation() {
    let dir =
        std::env::temp_dir().join(format!("simaris-test-devisolation-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = env!("CARGO_BIN_EXE_simaris");
    let output = Command::new(bin)
        .args(["add", "dev test", "--type", "fact"])
        .env("SIMARIS_HOME", &dir)
        .env("SIMARIS_ENV", "dev")
        .output()
        .expect("Failed to execute simaris");
    assert!(
        output.status.success(),
        "command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // DB should be created in the dev subdirectory
    assert!(
        dir.join("dev").join("sanctuary.db").exists(),
        "DB should be at {}/dev/sanctuary.db",
        dir.display()
    );
    // DB should NOT be at the base dir
    assert!(
        !dir.join("sanctuary.db").exists(),
        "DB should not be at base dir"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
