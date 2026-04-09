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
    let out = env.run_ok(&["link", &id_a, &id_b, "--rel", "related-to"]);
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
    env.run_ok(&["link", &id_a, &id_b, "--rel", "depends-on"]);

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
        stderr.to_lowercase().contains("not found"),
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
    env.run_ok(&["link", &id_a, &id_b, "--rel", "related-to"]);
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
