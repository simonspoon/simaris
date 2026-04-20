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
            .env("SIMARIS_CLAUDE_AGENTS_DIR", self.agents_dir())
            .output()
            .expect("Failed to execute simaris")
    }

    fn agents_dir(&self) -> std::path::PathBuf {
        self.dir.join("claude-agents")
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

/// Extract UUID from output like "Added unit 019375a2-..." or "Dropped item 019375a2-..."
fn extract_id(output: &str) -> String {
    output.split_whitespace().last().unwrap().to_string()
}

/// First 8 chars of a UUID, matching display::short_id
fn short_id(id: &str) -> &str {
    if id.len() >= 8 { &id[..8] } else { id }
}

/// Assert that a string looks like a UUIDv7 (36 chars, hex + hyphens, 8-4-4-4-12)
fn assert_uuid_format(id: &str) {
    assert_eq!(
        id.len(),
        36,
        "UUID should be 36 chars, got {}: {id}",
        id.len()
    );
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(
        parts.len(),
        5,
        "UUID should have 5 dash-separated parts: {id}"
    );
    assert_eq!(parts[0].len(), 8, "UUID part 1 should be 8 chars: {id}");
    assert_eq!(parts[1].len(), 4, "UUID part 2 should be 4 chars: {id}");
    assert_eq!(parts[2].len(), 4, "UUID part 3 should be 4 chars: {id}");
    assert_eq!(parts[3].len(), 4, "UUID part 4 should be 4 chars: {id}");
    assert_eq!(parts[4].len(), 12, "UUID part 5 should be 12 chars: {id}");
    for part in &parts {
        assert!(
            part.chars().all(|c| c.is_ascii_hexdigit()),
            "UUID parts should be hex: {id}"
        );
    }
}

#[test]
fn test_add_command() {
    let env = TestEnv::new("add");
    let out = env.run_ok(&["add", "hello world", "--type", "fact"]);
    assert!(out.starts_with("Added unit "), "got: {out}");
    let id = extract_id(&out);
    assert_uuid_format(&id);
}

#[test]
fn test_show_command() {
    let env = TestEnv::new("show");
    let out = env.run_ok(&[
        "add",
        "some knowledge",
        "--type",
        "principle",
        "--source",
        "test",
    ]);
    let id = extract_id(&out);
    let out = env.run_ok(&["show", &id]);
    assert!(out.contains("some knowledge"), "got: {out}");
    assert!(out.contains("principle"), "got: {out}");
    assert!(out.contains("test"), "got: {out}");
}

#[test]
fn test_link_command() {
    let env = TestEnv::new("link");
    let out_a = env.run_ok(&["add", "unit a", "--type", "fact"]);
    let id_a = extract_id(&out_a);
    let out_b = env.run_ok(&["add", "unit b", "--type", "idea"]);
    let id_b = extract_id(&out_b);
    let out = env.run_ok(&["link", &id_a, &id_b, "--rel", "related_to"]);
    assert!(
        out.contains(&format!("Linked {id_a} -> {id_b}")),
        "got: {out}"
    );
}

#[test]
fn test_show_with_links() {
    let env = TestEnv::new("showlinks");
    let out_a = env.run_ok(&["add", "unit a", "--type", "fact"]);
    let id_a = extract_id(&out_a);
    let out_b = env.run_ok(&["add", "unit b", "--type", "idea"]);
    let id_b = extract_id(&out_b);
    env.run_ok(&["link", &id_a, &id_b, "--rel", "depends_on"]);

    let out = env.run_ok(&["show", &id_a]);
    assert!(
        out.contains(&format!("-> {id_b} (depends_on)")),
        "got: {out}"
    );

    let out = env.run_ok(&["show", &id_b]);
    assert!(
        out.contains(&format!("<- {id_a} (depends_on)")),
        "got: {out}"
    );
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
    assert!(
        parsed["id"].is_string(),
        "id should be a string, got: {out}"
    );
    let id = parsed["id"].as_str().unwrap();
    assert_uuid_format(id);

    let out = env.run_ok(&["--json", "show", id]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(parsed["unit"]["content"], "json test");
    assert_eq!(parsed["unit"]["type"], "fact");
}

#[test]
fn test_drop_command() {
    let env = TestEnv::new("drop");
    let out = env.run_ok(&["drop", "raw idea about caching"]);
    assert!(out.starts_with("Dropped item "), "got: {out}");
    let id = extract_id(&out);
    assert_uuid_format(&id);

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
    assert!(
        parsed[0]["id"].is_string(),
        "id should be a string, got: {out}"
    );
    assert!(parsed[0]["created"].is_string());
}

#[test]
fn test_promote_command() {
    let env = TestEnv::new("promote");
    let drop_out = env.run_ok(&["drop", "caching matters for perf"]);
    let inbox_id = extract_id(&drop_out);

    let out = env.run_ok(&["promote", &inbox_id, "--type", "fact"]);
    assert!(out.starts_with("Added unit "), "got: {out}");
    let unit_id = extract_id(&out);
    assert_uuid_format(&unit_id);

    let out = env.run_ok(&["show", &unit_id]);
    assert!(out.contains("caching matters for perf"), "got: {out}");
    assert!(out.contains("fact"), "got: {out}");

    let out = env.run_ok(&["inbox"]);
    assert!(out.contains("Inbox is empty."), "got: {out}");
}

#[test]
fn test_promote_nonexistent_id() {
    let env = TestEnv::new("promotebad");
    let fake_uuid = "00000000-0000-0000-0000-000000000000";
    let output = env.run(&["promote", fake_uuid, "--type", "fact"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("Inbox item {fake_uuid} not found")),
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
fn test_aspect_type() {
    let env = TestEnv::new("aspect");
    let out = env.run_ok(&[
        "add",
        "Code review aspect — skeptical, thorough, read-only",
        "--type",
        "aspect",
        "--tags",
        "code-review,quality",
    ]);
    let id = extract_id(&out);

    // show displays correct type
    let show = env.run_ok(&["show", &id]);
    assert!(show.contains("aspect"), "got: {show}");
    assert!(show.contains("skeptical"), "got: {show}");

    // list --type aspect filters correctly
    env.run_ok(&["add", "some fact", "--type", "fact"]);
    let list = env.run_ok(&["list", "--type", "aspect"]);
    assert!(list.contains("skeptical"), "got: {list}");
    assert!(!list.contains("some fact"), "got: {list}");

    // search --type aspect filters correctly
    let search = env.run_ok(&["search", "review", "--type", "aspect"]);
    assert!(search.contains("skeptical"), "got: {search}");
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
fn test_search_type_filter() {
    let env = TestEnv::new("searchtypefilter");
    env.run_ok(&["add", "deploy procedure for cargo", "--type", "procedure"]);
    env.run_ok(&["add", "cargo is a build tool", "--type", "fact"]);
    let out = env.run_ok(&["search", "cargo", "--type", "procedure"]);
    assert!(out.contains("deploy procedure"), "got: {out}");
    assert!(!out.contains("build tool"), "got: {out}");
}

#[test]
fn test_search_type_filter_json() {
    let env = TestEnv::new("searchtypejson");
    env.run_ok(&["add", "deploy procedure for cargo", "--type", "procedure"]);
    env.run_ok(&["add", "cargo is a build tool", "--type", "fact"]);
    let out = env.run_ok(&["--json", "search", "cargo", "--type", "procedure"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    let arr = parsed.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["type"], "procedure");
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
    assert!(
        parsed[0]["id"].is_string(),
        "id should be a string, got: {out}"
    );
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

#[test]
fn test_ask_empty_store() {
    let env = TestEnv::new("askempty");
    // Ask on a fresh store with no units — should handle gracefully without calling LLM
    let out = env.run_ok(&["ask", "what is rust?"]);
    assert!(
        out.contains("No knowledge found"),
        "should report no knowledge on empty store, got: {out}"
    );
}

#[test]
fn test_ask_empty_store_json() {
    let env = TestEnv::new("askemptyjson");
    let out = env.run_ok(&["--json", "ask", "what is rust?"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(parsed["query"], "what is rust?");
    assert!(
        parsed["units"].is_array(),
        "should have units array, got: {out}"
    );
    assert_eq!(parsed["units"].as_array().unwrap().len(), 0);
    assert!(parsed["units_used"].is_array());
    assert_eq!(parsed["units_used"].as_array().unwrap().len(), 0);
    // response should not be present (skip_serializing_if = None)
    assert!(
        parsed["response"].is_null(),
        "response should be absent when not synthesizing, got: {out}"
    );
}

#[test]
fn test_ask_debug_flag() {
    let env = TestEnv::new("askdebug");
    let output = env.run(&["--debug", "ask", "what is rust?"]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("PHASE 1"),
        "debug output should contain PHASE 1, got stderr: {stderr}"
    );
    // stdout should still contain the normal response
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No knowledge found"),
        "stdout should contain normal response, got: {stdout}"
    );
}

#[test]
fn test_mark_command() {
    let env = TestEnv::new("mark");
    let out = env.run_ok(&["add", "test unit", "--type", "fact"]);
    let id = extract_id(&out);
    let out = env.run_ok(&["mark", &id, "--kind", "wrong"]);
    assert!(
        out.contains(&format!("Marked unit {id} as wrong")),
        "got: {out}"
    );
    assert!(out.contains("confidence: 0.80"), "got: {out}");
}

#[test]
fn test_mark_json_output() {
    let env = TestEnv::new("markjson");
    let out = env.run_ok(&["add", "test unit", "--type", "fact"]);
    let id = extract_id(&out);
    let out = env.run_ok(&["--json", "mark", &id, "--kind", "outdated"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(v["id"].as_str().unwrap(), id, "id should match");
    assert_eq!(v["mark"], "outdated");
    assert!(v["confidence"].as_f64().unwrap() < 1.0);
}

#[test]
fn test_mark_nonexistent() {
    let env = TestEnv::new("marknonexist");
    let fake_uuid = "00000000-0000-0000-0000-000000000000";
    let output = env.run(&["mark", fake_uuid, "--kind", "used"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No unit or slug matches") && stderr.contains(fake_uuid),
        "got stderr: {stderr}"
    );
}

#[test]
fn test_mark_confidence_accumulation() {
    let env = TestEnv::new("markaccum");
    let out = env.run_ok(&["add", "test unit", "--type", "fact"]);
    let id = extract_id(&out);
    env.run_ok(&["mark", &id, "--kind", "wrong"]); // 1.0 -> 0.8
    let out = env.run_ok(&["mark", &id, "--kind", "helpful"]); // 0.8 -> 0.9
    assert!(out.contains("0.90"), "got: {out}");
}

#[test]
fn test_mark_clamping() {
    let env = TestEnv::new("markclamp");
    let out = env.run_ok(&["add", "test unit", "--type", "fact"]);
    let id = extract_id(&out);
    for _ in 0..10 {
        env.run_ok(&["mark", &id, "--kind", "wrong"]);
    }
    let out = env.run_ok(&["show", &id]);
    assert!(out.contains("confidence: 0  verified"), "got: {out}");
}

#[test]
fn test_mark_no_kind() {
    let env = TestEnv::new("marknokind");
    let fake_uuid = "00000000-0000-0000-0000-000000000000";
    let output = env.run(&["mark", fake_uuid]);
    assert!(!output.status.success());
}

#[test]
fn test_ask_debug_json() {
    let env = TestEnv::new("askdebugjson");
    let out = env.run_ok(&["--json", "--debug", "ask", "what is rust?"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(parsed["query"], "what is rust?");
    assert!(
        parsed["debug"].is_object(),
        "JSON should include debug field when --debug is set, got: {out}"
    );
    assert!(
        parsed["debug"]["fts_query"].is_string(),
        "should have fts_query field, got: {out}"
    );
    assert!(parsed["debug"]["matches_per_query"].is_object());
    assert!(parsed["debug"]["total_gathered"].is_number());
    assert!(parsed["debug"]["filter_kept"].is_number());
    assert!(parsed["debug"]["filter_total"].is_number());
}

#[test]
fn test_scan_empty() {
    let env = TestEnv::new("scanempty");
    let out = env.run_ok(&["scan"]);
    assert!(out.contains("No issues found."), "got: {out}");
}

#[test]
fn test_scan_low_confidence() {
    let env = TestEnv::new("scanlowconf");
    let out = env.run_ok(&["add", "dubious claim", "--type", "fact"]);
    let id = extract_id(&out);
    // Mark wrong 3 times: 1.0 -> 0.8 -> 0.6 -> 0.4
    env.run_ok(&["mark", &id, "--kind", "wrong"]);
    env.run_ok(&["mark", &id, "--kind", "wrong"]);
    env.run_ok(&["mark", &id, "--kind", "wrong"]);
    let out = env.run_ok(&["scan"]);
    assert!(out.contains("Low confidence"), "got: {out}");
    assert!(out.contains(&format!("[{}]", short_id(&id))), "got: {out}");
}

#[test]
fn test_scan_contradictions() {
    let env = TestEnv::new("scancontradictions");
    let out_a = env.run_ok(&["add", "the sky is blue", "--type", "fact"]);
    let id_a = extract_id(&out_a);
    let out_b = env.run_ok(&["add", "the sky is green", "--type", "fact"]);
    let id_b = extract_id(&out_b);
    env.run_ok(&["link", &id_a, &id_b, "--rel", "contradicts"]);
    let out = env.run_ok(&["scan"]);
    assert!(out.contains("Contradictions"), "got: {out}");
    assert!(
        out.contains(&format!("[{}]", short_id(&id_a))),
        "got: {out}"
    );
    assert!(
        out.contains(&format!("[{}]", short_id(&id_b))),
        "got: {out}"
    );
}

#[test]
fn test_scan_orphans() {
    let env = TestEnv::new("scanorphans");
    let out_a = env.run_ok(&["add", "linked unit a", "--type", "fact"]);
    let id_a = extract_id(&out_a);
    let out_b = env.run_ok(&["add", "linked unit b", "--type", "fact"]);
    let id_b = extract_id(&out_b);
    let out_c = env.run_ok(&["add", "lonely unit c", "--type", "fact"]);
    let id_c = extract_id(&out_c);
    env.run_ok(&["link", &id_a, &id_b, "--rel", "related_to"]);
    let out = env.run_ok(&["scan"]);
    assert!(out.contains("Orphans"), "got: {out}");
    assert!(
        out.contains(&format!("[{}]", short_id(&id_c))),
        "got: {out}"
    );
    // Units a and b are linked, should NOT appear in orphans section
    // Note: UUIDv7 short_ids may collide for rapidly-created units (same ms prefix),
    // so we check by content text instead.
    let orphans_section = out
        .split("Orphans:")
        .nth(1)
        .expect("Orphans section missing");
    assert!(
        !orphans_section.contains("linked unit a"),
        "unit a should not be orphan, got: {out}"
    );
    assert!(
        !orphans_section.contains("linked unit b"),
        "unit b should not be orphan, got: {out}"
    );
}

#[test]
fn test_scan_json() {
    let env = TestEnv::new("scanjson");
    let out = env.run_ok(&["add", "shaky knowledge", "--type", "fact"]);
    let id = extract_id(&out);
    // Mark wrong 3 times: 1.0 -> 0.8 -> 0.6 -> 0.4 (low confidence + negative marks)
    env.run_ok(&["mark", &id, "--kind", "wrong"]);
    env.run_ok(&["mark", &id, "--kind", "wrong"]);
    env.run_ok(&["mark", &id, "--kind", "wrong"]);
    let out = env.run_ok(&["--json", "scan"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert!(parsed["low_confidence"].is_array(), "got: {out}");
    assert!(parsed["negative_marks"].is_array(), "got: {out}");
    assert!(parsed["contradictions"].is_array(), "got: {out}");
    assert!(parsed["orphans"].is_array(), "got: {out}");
    assert!(parsed["stale"].is_array(), "got: {out}");
    // Unit should appear in low_confidence
    let low_conf = parsed["low_confidence"].as_array().unwrap();
    assert!(
        low_conf.iter().any(|u| u["id"].as_str() == Some(&*id)),
        "unit should be in low_confidence, got: {out}"
    );
    // Unit should appear in negative_marks
    let neg_marks = parsed["negative_marks"].as_array().unwrap();
    assert!(
        neg_marks.iter().any(|u| u["id"].as_str() == Some(&*id)),
        "unit should be in negative_marks, got: {out}"
    );
}

#[test]
fn test_scan_stale_days() {
    let env = TestEnv::new("scanstale");
    let out = env.run_ok(&["add", "ancient knowledge", "--type", "fact"]);
    let id = extract_id(&out);
    // Backdate the unit's created timestamp via SQLite directly
    let db_path = env.dir.join("sanctuary.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    conn.execute(
        "UPDATE units SET created = datetime('now', '-91 days') WHERE id = ?1",
        rusqlite::params![id],
    )
    .unwrap();
    drop(conn);
    let out = env.run_ok(&["scan"]);
    assert!(out.contains("Stale"), "got: {out}");
    assert!(out.contains(&format!("[{}]", short_id(&id))), "got: {out}");
}

#[test]
fn test_add_auto_links() {
    let env = TestEnv::new("autolink");
    let out_a = env.run_ok(&[
        "add",
        "rust cli tool",
        "--type",
        "fact",
        "--tags",
        "rust,cli,tools",
    ]);
    let id_a = extract_id(&out_a);
    // First unit has no peers to link to
    assert!(!out_a.contains("auto-linked"), "got: {out_a}");

    let out_b = env.run_ok(&[
        "add",
        "rust cli testing",
        "--type",
        "procedure",
        "--tags",
        "rust,cli,testing",
    ]);
    assert!(
        out_b.contains("auto-linked to 1 existing unit"),
        "got: {out_b}"
    );

    // extract_id from first line only (second line is the auto-link message)
    let id_b = extract_id(out_b.lines().next().unwrap());

    // Verify the link exists via show
    let show = env.run_ok(&["--json", "show", &id_b]);
    assert!(
        show.contains(&id_a),
        "show should reference unit A, got: {show}"
    );
    assert!(
        show.contains("related_to"),
        "should have related_to link, got: {show}"
    );
}

#[test]
fn test_add_no_auto_link_one_shared_tag() {
    let env = TestEnv::new("noautolink");
    env.run_ok(&["add", "python script", "--type", "fact", "--tags", "python"]);
    let out = env.run_ok(&["add", "python lib", "--type", "fact", "--tags", "python"]);
    assert!(
        !out.contains("auto-linked"),
        "should not auto-link with only 1 shared tag, got: {out}"
    );
}

#[test]
fn test_migration_runs_on_first_command() {
    let env = TestEnv::new("migratefirst");
    env.run_ok(&["add", "x", "--type", "fact"]);

    let db_path = env.dir.join("sanctuary.db");
    let conn = rusqlite::Connection::open(&db_path).expect("open sanctuary.db");

    let version: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 3, "expected user_version=3, got {version}");

    let slugs_present: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='slugs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(slugs_present, 1, "expected slugs table to exist");

    let slugs_index: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='index' AND name='idx_slugs_unit'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(slugs_index, 1, "expected idx_slugs_unit to exist");
}

#[test]
fn test_existing_v2_db_upgrades_on_launch() {
    let env = TestEnv::new("v2upgrade");
    let db_path = env.dir.join("sanctuary.db");

    // Hand-roll a v2 DB on disk, pre-seeded with a row in each table.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE units (
                id          TEXT PRIMARY KEY,
                content     TEXT NOT NULL,
                type        TEXT NOT NULL CHECK(type IN ('fact','procedure','principle','preference','lesson','idea','aspect')),
                source      TEXT NOT NULL DEFAULT 'inbox',
                confidence  REAL NOT NULL DEFAULT 1.0,
                verified    INTEGER NOT NULL DEFAULT 0,
                tags        TEXT NOT NULL DEFAULT '[]',
                conditions  TEXT NOT NULL DEFAULT '{}',
                created     TEXT NOT NULL DEFAULT (datetime('now')),
                updated     TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE links (
                from_id      TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
                to_id        TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
                relationship TEXT NOT NULL CHECK(relationship IN (
                                 'related_to','part_of','depends_on',
                                 'contradicts','supersedes','sourced_from')),
                PRIMARY KEY (from_id, to_id, relationship)
            );
            CREATE INDEX idx_links_to ON links(to_id);
            CREATE TABLE inbox (
                id      TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                source  TEXT NOT NULL DEFAULT 'cli',
                created TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE marks (
                id       TEXT PRIMARY KEY,
                unit_id  TEXT NOT NULL REFERENCES units(id) ON DELETE CASCADE,
                kind     TEXT NOT NULL CHECK(kind IN ('used','wrong','outdated','helpful')),
                created  TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX idx_marks_unit ON marks(unit_id);
            CREATE VIRTUAL TABLE units_fts USING fts5(uuid, content, type, tags, source);
            CREATE TRIGGER units_ai AFTER INSERT ON units BEGIN
                INSERT INTO units_fts(uuid, content, type, tags, source)
                VALUES (new.id, new.content, new.type, new.tags, new.source);
            END;
            CREATE TRIGGER units_ad AFTER DELETE ON units BEGIN
                DELETE FROM units_fts WHERE uuid = old.id;
            END;
            CREATE TRIGGER units_au AFTER UPDATE ON units BEGIN
                DELETE FROM units_fts WHERE uuid = old.id;
                INSERT INTO units_fts(uuid, content, type, tags, source)
                VALUES (new.id, new.content, new.type, new.tags, new.source);
            END;
            INSERT INTO units (id, content, type) VALUES
                ('0193-seed-unit-aaaa-aaaaaaaaaaaa', 'pre-existing fact', 'fact');
            PRAGMA user_version = 2;",
        )
        .unwrap();
    }

    // Launch simaris — `list` is pure read, triggers connect() migration ladder.
    let out = env.run_ok(&["list"]);
    assert!(
        out.contains("pre-existing fact"),
        "seeded unit should be intact: {out}"
    );

    // Re-open the upgraded DB directly and verify state.
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let version: i32 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 3);

    let slugs_present: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='slugs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(slugs_present, 1);

    let seeded: i64 = conn
        .query_row(
            "SELECT count(*) FROM units WHERE id='0193-seed-unit-aaaa-aaaaaaaaaaaa'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seeded, 1, "pre-existing unit should survive migration");

    // Backup file should have been written by create_backup.
    let backup_dir = env.dir.join("backups");
    let backups: Vec<_> = std::fs::read_dir(&backup_dir)
        .expect("backups dir should exist")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.starts_with("sanctuary-") && n.ends_with(".db"))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        !backups.is_empty(),
        "expected at least one backup file in {backup_dir:?}"
    );
}

#[test]
fn test_slug_set_and_list_roundtrip() {
    let env = TestEnv::new("slug-roundtrip");
    let out = env.run_ok(&["add", "alpha unit", "--type", "fact"]);
    let id = extract_id(&out);
    let set_out = env.run_ok(&["slug", "set", "alpha", &id]);
    assert!(
        set_out.contains("alpha") && set_out.contains(&id),
        "set stdout missing slug/id: {set_out}"
    );
    let list_out = env.run_ok(&["slug", "list"]);
    assert!(list_out.contains("alpha"), "list missing slug: {list_out}");
    assert!(list_out.contains(&id), "list missing id: {list_out}");
}

#[test]
fn test_slug_list_empty() {
    let env = TestEnv::new("slug-list-empty");
    let out = env.run_ok(&["slug", "list"]);
    assert!(out.contains("No slugs."), "got: {out}");
}

#[test]
fn test_slug_set_rebinds_same_slug_to_new_unit() {
    let env = TestEnv::new("slug-rebind");
    let out_a = env.run_ok(&["add", "unit a", "--type", "fact"]);
    let id_a = extract_id(&out_a);
    let out_b = env.run_ok(&["add", "unit b", "--type", "fact"]);
    let id_b = extract_id(&out_b);

    env.run_ok(&["slug", "set", "ptr", &id_a]);
    env.run_ok(&["slug", "set", "ptr", &id_b]);

    let out = env.run_ok(&["--json", "slug", "list"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON array");
    assert_eq!(parsed.len(), 1, "expected one slug row: {out}");
    assert_eq!(parsed[0]["slug"], "ptr");
    assert_eq!(parsed[0]["unit_id"], id_b);
}

#[test]
fn test_slug_unset_existing_returns_success() {
    let env = TestEnv::new("slug-unset-hit");
    let out = env.run_ok(&["add", "gone unit", "--type", "fact"]);
    let id = extract_id(&out);
    env.run_ok(&["slug", "set", "gone", &id]);
    let unset_out = env.run_ok(&["slug", "unset", "gone"]);
    assert!(unset_out.contains("Unset slug 'gone'"), "got: {unset_out}");
    let list_out = env.run_ok(&["slug", "list"]);
    assert!(list_out.contains("No slugs."), "got: {list_out}");
}

#[test]
fn test_slug_unset_nonexistent_is_noop() {
    let env = TestEnv::new("slug-unset-miss");
    let text_out = env.run(&["slug", "unset", "nope"]);
    assert!(
        text_out.status.success(),
        "unset miss should exit 0: stderr={}",
        String::from_utf8_lossy(&text_out.stderr)
    );
    let json_out = env.run_ok(&["--json", "slug", "unset", "nope"]);
    let parsed: serde_json::Value = serde_json::from_str(&json_out).expect("valid JSON");
    assert_eq!(parsed["removed"], false, "got: {json_out}");
    assert_eq!(parsed["unset"], "nope", "got: {json_out}");
}

#[test]
fn test_slug_set_invalid_slug_rejected() {
    let env = TestEnv::new("slug-invalid");
    let out = env.run_ok(&["add", "host unit", "--type", "fact"]);
    let id = extract_id(&out);

    for bad in ["", "Bad!", "1foo", "UPPER"] {
        let result = env.run(&["slug", "set", bad, &id]);
        assert!(
            !result.status.success(),
            "slug '{bad}' should have been rejected"
        );
        let stderr = String::from_utf8_lossy(&result.stderr).to_lowercase();
        assert!(
            stderr.contains("slug"),
            "stderr should mention slug for '{bad}': {stderr}"
        );
    }
}

#[test]
fn test_slug_set_unknown_unit_id_rejected() {
    let env = TestEnv::new("slug-unknown-id");
    let result = env.run(&["slug", "set", "foo", "00000000-0000-0000-0000-000000000000"]);
    assert!(
        !result.status.success(),
        "unknown unit id should fail; stdout={}",
        String::from_utf8_lossy(&result.stdout)
    );
    let stderr = String::from_utf8_lossy(&result.stderr).to_lowercase();
    assert!(
        stderr.contains("not found"),
        "stderr should say not found: {stderr}"
    );
}

#[test]
fn test_slug_set_json_output() {
    let env = TestEnv::new("slug-set-json");
    let out = env.run_ok(&["add", "jset unit", "--type", "fact"]);
    let id = extract_id(&out);
    let json_out = env.run_ok(&["--json", "slug", "set", "alpha", &id]);
    let parsed: serde_json::Value = serde_json::from_str(&json_out).expect("valid JSON");
    assert_eq!(parsed["slug"], "alpha", "got: {json_out}");
    assert_eq!(parsed["unit_id"], id, "got: {json_out}");
}

#[test]
fn test_slug_list_json_output() {
    let env = TestEnv::new("slug-list-json");
    let out = env.run_ok(&["add", "multi unit", "--type", "fact"]);
    let id = extract_id(&out);
    env.run_ok(&["slug", "set", "a", &id]);
    env.run_ok(&["slug", "set", "z", &id]);
    let json_out = env.run_ok(&["--json", "slug", "list"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json_out).expect("valid JSON array");
    assert_eq!(parsed.len(), 2, "got: {json_out}");
    assert_eq!(parsed[0]["slug"], "a");
    assert_eq!(parsed[1]["slug"], "z");
}

#[test]
fn test_slug_unset_json_output() {
    let env = TestEnv::new("slug-unset-json");
    let out = env.run_ok(&["add", "jtarget", "--type", "fact"]);
    let id = extract_id(&out);
    env.run_ok(&["slug", "set", "hit", &id]);

    let hit_out = env.run_ok(&["--json", "slug", "unset", "hit"]);
    let parsed_hit: serde_json::Value = serde_json::from_str(&hit_out).expect("valid JSON");
    assert_eq!(parsed_hit["removed"], true, "got: {hit_out}");
    assert_eq!(parsed_hit["unset"], "hit", "got: {hit_out}");

    let miss_out = env.run_ok(&["--json", "slug", "unset", "miss"]);
    let parsed_miss: serde_json::Value = serde_json::from_str(&miss_out).expect("valid JSON");
    assert_eq!(parsed_miss["removed"], false, "got: {miss_out}");
    assert_eq!(parsed_miss["unset"], "miss", "got: {miss_out}");
}

#[test]
fn test_show_resolves_slug() {
    let env = TestEnv::new("slug-wire-show");
    let add_out = env.run_ok(&["add", "showable knowledge", "--type", "principle"]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "shoe", &id]);
    let out = env.run_ok(&["show", "shoe"]);
    assert!(out.contains("showable knowledge"), "got: {out}");
    assert!(out.contains("principle"), "got: {out}");
}

#[test]
fn test_edit_resolves_slug() {
    let env = TestEnv::new("slug-wire-edit");
    let add_out = env.run_ok(&["add", "old text", "--type", "fact"]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "editme", &id]);
    let out = env.run_ok(&["edit", "editme", "--content", "new text"]);
    assert!(out.contains("new text"), "got: {out}");
    let show = env.run_ok(&["show", &id]);
    assert!(show.contains("new text"), "got: {show}");
}

#[test]
fn test_link_resolves_from_slug() {
    let env = TestEnv::new("slug-wire-link-from");
    let out_a = env.run_ok(&["add", "unit a", "--type", "fact"]);
    let id_a = extract_id(&out_a);
    let out_b = env.run_ok(&["add", "unit b", "--type", "idea"]);
    let id_b = extract_id(&out_b);
    env.run_ok(&["slug", "set", "alpha", &id_a]);
    let out = env.run_ok(&["link", "alpha", &id_b, "--rel", "related_to"]);
    assert!(
        out.contains(&format!("Linked {id_a} -> {id_b}")),
        "got: {out}"
    );
}

#[test]
fn test_link_resolves_to_slug() {
    let env = TestEnv::new("slug-wire-link-to");
    let out_a = env.run_ok(&["add", "unit a", "--type", "fact"]);
    let id_a = extract_id(&out_a);
    let out_b = env.run_ok(&["add", "unit b", "--type", "idea"]);
    let id_b = extract_id(&out_b);
    env.run_ok(&["slug", "set", "beta", &id_b]);
    let out = env.run_ok(&["link", &id_a, "beta", "--rel", "depends_on"]);
    assert!(
        out.contains(&format!("Linked {id_a} -> {id_b}")),
        "got: {out}"
    );
}

#[test]
fn test_link_resolves_both_slugs() {
    let env = TestEnv::new("slug-wire-link-both");
    let out_a = env.run_ok(&["add", "unit a", "--type", "fact"]);
    let id_a = extract_id(&out_a);
    let out_b = env.run_ok(&["add", "unit b", "--type", "idea"]);
    let id_b = extract_id(&out_b);
    env.run_ok(&["slug", "set", "aa", &id_a]);
    env.run_ok(&["slug", "set", "bb", &id_b]);
    let out = env.run_ok(&["link", "aa", "bb", "--rel", "part_of"]);
    assert!(
        out.contains(&format!("Linked {id_a} -> {id_b}")),
        "got: {out}"
    );
}

#[test]
fn test_delete_resolves_slug() {
    let env = TestEnv::new("slug-wire-delete");
    let add_out = env.run_ok(&["add", "doomed unit", "--type", "fact"]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "bye", &id]);
    let out = env.run_ok(&["delete", "bye"]);
    assert!(out.contains(&id), "delete should echo resolved id: {out}");
    // Verify unit really gone
    let result = env.run(&["show", &id]);
    assert!(!result.status.success(), "unit should be gone after delete");
}

#[test]
fn test_mark_resolves_slug() {
    let env = TestEnv::new("slug-wire-mark");
    let add_out = env.run_ok(&["add", "mark me", "--type", "fact"]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "mk", &id]);
    let out = env.run_ok(&["--json", "mark", "mk", "--kind", "used"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    assert_eq!(v["id"].as_str().unwrap(), id, "id should be resolved UUID");
    assert_eq!(v["mark"], "used");
    // used delta = +0.05, but confidence already clamped at 1.0, so stays 1.0
    assert_eq!(v["confidence"].as_f64().unwrap(), 1.0);

    // Knock confidence down first, then verify +0.05 delta applies through slug
    env.run_ok(&["mark", &id, "--kind", "wrong"]); // 1.0 -> 0.8
    let out = env.run_ok(&["--json", "mark", "mk", "--kind", "used"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
    let conf = v["confidence"].as_f64().unwrap();
    assert!(
        (conf - 0.85).abs() < 1e-9,
        "expected 0.85 after +0.05 delta from slug, got {conf}"
    );
}

#[test]
fn test_unknown_slug_rejected_per_command() {
    let env = TestEnv::new("slug-wire-unknown");
    // Seed one unit so Link has an existing id to pair against (still fails on
    // the unknown side), but this is defensive — both ids should fail first.
    let out = env.run_ok(&["add", "seed", "--type", "fact"]);
    let real_id = extract_id(&out);

    let invocations: Vec<Vec<&str>> = vec![
        vec!["show", "ghost"],
        vec!["edit", "ghost", "--content", "nope"],
        vec!["link", "ghost", &real_id, "--rel", "related_to"],
        vec!["delete", "ghost"],
        vec!["mark", "ghost", "--kind", "used"],
    ];
    for args in invocations {
        let result = env.run(&args);
        assert!(
            !result.status.success(),
            "cmd should fail for unknown slug: {args:?}"
        );
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            stderr.contains("No unit or slug matches"),
            "stderr should mention resolver msg for {args:?}: {stderr}"
        );
        assert!(
            stderr.contains("ghost"),
            "stderr should include input verbatim for {args:?}: {stderr}"
        );
    }
}

#[test]
fn test_raw_uuid_still_works_after_slug_wiring() {
    let env = TestEnv::new("slug-wire-raw-uuid");
    let out_a = env.run_ok(&["add", "raw a", "--type", "fact"]);
    let id_a = extract_id(&out_a);
    let out_b = env.run_ok(&["add", "raw b", "--type", "idea"]);
    let id_b = extract_id(&out_b);

    // show by raw UUID
    let show = env.run_ok(&["show", &id_a]);
    assert!(show.contains("raw a"), "show raw uuid: {show}");

    // edit by raw UUID
    let edit = env.run_ok(&["edit", &id_a, "--content", "edited raw a"]);
    assert!(edit.contains("edited raw a"), "edit raw uuid: {edit}");

    // link by raw UUIDs
    let link = env.run_ok(&["link", &id_a, &id_b, "--rel", "related_to"]);
    assert!(
        link.contains(&format!("Linked {id_a} -> {id_b}")),
        "link raw uuids: {link}"
    );

    // mark by raw UUID
    let mark = env.run_ok(&["mark", &id_a, "--kind", "wrong"]);
    assert!(mark.contains(&id_a), "mark raw uuid: {mark}");

    // delete by raw UUID
    let del = env.run_ok(&["delete", &id_b]);
    assert!(del.contains(&id_b), "delete raw uuid: {del}");
}

#[test]
fn test_slug_missing_args() {
    let env = TestEnv::new("slug-missing-args");
    for args in [
        vec!["slug", "set", "foo"],
        vec!["slug", "set"],
        vec!["slug"],
    ] {
        let result = env.run(&args);
        assert!(
            !result.status.success(),
            "missing-args call should fail: {:?}",
            args
        );
    }
}

#[test]
fn test_show_text_includes_slugs_when_set() {
    let env = TestEnv::new("show-slugs-text");
    let add_out = env.run_ok(&["add", "slugged content", "--type", "fact"]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "beta", &id]);
    env.run_ok(&["slug", "set", "alpha", &id]);
    let out = env.run_ok(&["show", &id]);
    assert!(
        out.contains("Slugs: alpha, beta"),
        "expected alphabetical comma-space slugs line: {out}"
    );
}

#[test]
fn test_show_text_omits_slugs_line_when_none() {
    let env = TestEnv::new("show-slugs-text-none");
    let add_out = env.run_ok(&["add", "no slugs here", "--type", "fact"]);
    let id = extract_id(&add_out);
    let out = env.run_ok(&["show", &id]);
    assert!(
        !out.contains("Slugs:"),
        "should omit Slugs line when none set: {out}"
    );
}

#[test]
fn test_show_json_includes_slugs_array() {
    let env = TestEnv::new("show-slugs-json");
    let add_out = env.run_ok(&["add", "json slug content", "--type", "fact"]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "foo", &id]);
    let out = env.run_ok(&["--json", "show", &id]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    let slugs = parsed["slugs"].as_array().expect("top-level slugs array");
    assert_eq!(slugs.len(), 1, "expected single slug: {out}");
    assert_eq!(slugs[0], "foo", "got: {out}");
}

#[test]
fn test_show_json_slugs_empty_array_when_none() {
    let env = TestEnv::new("show-slugs-json-empty");
    let add_out = env.run_ok(&["add", "no slugs", "--type", "fact"]);
    let id = extract_id(&add_out);
    let out = env.run_ok(&["--json", "show", &id]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert!(
        parsed.get("slugs").is_some(),
        "slugs key must be present: {out}"
    );
    let slugs = parsed["slugs"].as_array().expect("slugs must be array");
    assert!(slugs.is_empty(), "slugs should be empty array: {out}");
}

#[test]
fn test_show_by_slug_renders_slugs_section() {
    let env = TestEnv::new("show-by-slug-renders");
    let add_out = env.run_ok(&["add", "slug resolver content", "--type", "fact"]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "myslug", &id]);
    let out = env.run_ok(&["show", "myslug"]);
    assert!(
        out.contains("Slugs: myslug"),
        "expected Slugs line when resolving via slug: {out}"
    );
}

#[test]
fn test_emit_writes_agent_file_for_slugged_aspect() {
    let env = TestEnv::new("emit-writes-slugged");
    let add_out = env.run_ok(&[
        "add",
        "# Worker\n\nHelper agent used for routine tasks.",
        "--type",
        "aspect",
    ]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "worker", &id]);

    let stdout = env.run_ok(&["emit", "--target", "claude-code", "--type", "aspect"]);
    assert!(stdout.contains("Written: 1"), "summary: {stdout}");
    assert!(stdout.contains("worker"), "summary lists slug: {stdout}");

    let file = env.agents_dir().join("worker.md");
    let body = std::fs::read_to_string(&file).expect("agent file must exist");
    assert!(body.starts_with("---\n"), "frontmatter starts: {body}");
    assert!(body.contains("\nname: worker\n"), "name line: {body}");
    assert!(
        body.contains("\ndescription: '# Worker'\n"),
        "description line: {body}"
    );
    assert!(
        body.contains("\nsimaris-managed: true\n"),
        "managed marker: {body}"
    );
    assert!(
        body.contains("\n---\n# Worker\n\nHelper agent used for routine tasks."),
        "body verbatim: {body}"
    );
}

#[test]
fn test_emit_skips_aspects_without_slug() {
    let env = TestEnv::new("emit-skips-slugless");
    let add_out = env.run_ok(&["add", "no slug content", "--type", "aspect"]);
    let id = extract_id(&add_out);

    let stdout = env.run_ok(&["emit", "--target", "claude-code", "--type", "aspect"]);
    assert!(stdout.contains("Written: 0"), "summary: {stdout}");
    assert!(stdout.contains("Skipped: 1"), "summary: {stdout}");
    assert!(
        stdout.contains(&id),
        "summary should list skipped UUID: {stdout}"
    );
}

#[test]
fn test_emit_rerun_overwrites_managed_file() {
    let env = TestEnv::new("emit-rerun-overwrite");
    let add_out = env.run_ok(&["add", "first body", "--type", "aspect"]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "agent_one", &id]);

    env.run_ok(&["emit", "--target", "claude-code", "--type", "aspect"]);
    env.run_ok(&["edit", &id, "--content", "second body"]);
    env.run_ok(&["emit", "--target", "claude-code", "--type", "aspect"]);

    let file = env.agents_dir().join("agent_one.md");
    let body = std::fs::read_to_string(&file).unwrap();
    assert!(body.contains("second body"), "rewrite body: {body}");
    assert!(!body.contains("first body"), "old body gone: {body}");
}

#[test]
fn test_emit_sweeps_managed_file_when_aspect_removed() {
    let env = TestEnv::new("emit-sweep");
    let add_out = env.run_ok(&["add", "going away", "--type", "aspect"]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "ghost", &id]);

    env.run_ok(&["emit", "--target", "claude-code", "--type", "aspect"]);
    let file = env.agents_dir().join("ghost.md");
    assert!(file.exists(), "initial emit writes file");

    env.run_ok(&["slug", "unset", "ghost"]);
    let stdout = env.run_ok(&["emit", "--target", "claude-code", "--type", "aspect"]);
    assert!(stdout.contains("Swept: 1"), "summary: {stdout}");
    assert!(!file.exists(), "sweep deletes orphaned managed file");
}

#[test]
fn test_emit_preserves_hand_authored_file() {
    let env = TestEnv::new("emit-preserves-hand");
    let add_out = env.run_ok(&["add", "managed body", "--type", "aspect"]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "managed_slug", &id]);

    std::fs::create_dir_all(env.agents_dir()).unwrap();
    let hand = env.agents_dir().join("hand.md");
    std::fs::write(&hand, "---\nname: hand\n---\nhuman wrote this\n").unwrap();

    env.run_ok(&["emit", "--target", "claude-code", "--type", "aspect"]);

    assert!(hand.exists(), "hand-authored file must survive emit");
    let body = std::fs::read_to_string(&hand).unwrap();
    assert!(
        body.contains("human wrote this"),
        "hand body intact: {body}"
    );
}

#[test]
fn test_emit_json_shape() {
    let env = TestEnv::new("emit-json");
    let add_out = env.run_ok(&["add", "json body", "--type", "aspect"]);
    let id = extract_id(&add_out);
    env.run_ok(&["slug", "set", "jsonner", &id]);

    let out = env.run_ok(&["--json", "emit", "--target", "claude-code", "--type", "aspect"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(parsed["written"], serde_json::json!(["jsonner"]));
    assert_eq!(parsed["swept"], serde_json::json!([]));
    assert_eq!(parsed["skipped_uuids"], serde_json::json!([]));
    assert!(parsed["target_dir"].is_string(), "target_dir string");
}
