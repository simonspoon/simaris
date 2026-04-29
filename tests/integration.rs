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

    fn run_with_env(&self, args: &[&str], envs: &[(&str, &str)]) -> std::process::Output {
        let bin = env!("CARGO_BIN_EXE_simaris");
        let mut cmd = Command::new(bin);
        cmd.args(args)
            .env("SIMARIS_HOME", &self.dir)
            .env("SIMARIS_CLAUDE_AGENTS_DIR", self.agents_dir());
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.output().expect("Failed to execute simaris")
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

    // Default list output is LEAN: headline, no content.
    let out = env.run_ok(&["--json", "list"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON array");
    assert_eq!(parsed.len(), 2);
    assert!(
        parsed.iter().all(|u| u.get("content").is_none()),
        "default list JSON must omit `content`, got: {out}"
    );
    assert!(parsed.iter().any(|u| u["headline"] == "fact one"));
    assert!(parsed.iter().any(|u| u["headline"] == "idea one"));
    for u in &parsed {
        assert!(u["id"].is_string(), "id should be a string, got: {out}");
        assert!(u["type"].is_string());
        assert!(u["tags"].is_array());
        assert!(u["source"].is_string());
        assert!(u["confidence"].is_number());
        // slug is null when no slug bound
        assert!(u.get("slug").is_some(), "slug key must be present");
    }
}

#[test]
fn test_list_json_full_restores_content() {
    let env = TestEnv::new("listjsonfull");
    env.run_ok(&["add", "fact one", "--type", "fact", "--source", "test"]);

    let out = env.run_ok(&["--json", "list", "--full"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON array");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0]["content"], "fact one");
    assert!(parsed[0]["id"].is_string());
}

#[test]
fn test_search_json_default_lean() {
    let env = TestEnv::new("searchjsonlean");
    env.run_ok(&[
        "add",
        "caching improves performance\nsecond line detail",
        "--type",
        "fact",
    ]);

    let out = env.run_ok(&["--json", "search", "caching"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON array");
    assert_eq!(parsed.len(), 1);
    assert!(
        parsed[0].get("content").is_none(),
        "default search JSON must omit `content`, got: {out}"
    );
    assert_eq!(parsed[0]["headline"], "caching improves performance");
    // Body second line must not leak via headline.
    assert!(
        !parsed[0]["headline"]
            .as_str()
            .unwrap()
            .contains("second line"),
        "headline must be first line only, got: {out}"
    );
}

#[test]
fn test_search_json_full_restores_content() {
    let env = TestEnv::new("searchjsonfull");
    env.run_ok(&[
        "add",
        "caching improves performance\nsecond line detail",
        "--type",
        "fact",
    ]);

    let out = env.run_ok(&["--json", "search", "caching", "--full"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON array");
    assert_eq!(parsed.len(), 1);
    let content = parsed[0]["content"].as_str().unwrap();
    assert!(content.contains("caching improves performance"));
    assert!(content.contains("second line detail"));
}

#[test]
fn test_list_default_text_omits_body() {
    let env = TestEnv::new("listtextlean");
    // Multi-line body; default text output must show first line only.
    env.run_ok(&[
        "add",
        "first line headline\nsecond line body content that must not appear",
        "--type",
        "fact",
    ]);
    let out = env.run_ok(&["list"]);
    assert!(out.contains("first line headline"), "got: {out}");
    assert!(
        !out.contains("second line body content"),
        "default text output leaked body, got: {out}"
    );
}

#[test]
fn test_list_full_text_shows_body() {
    let env = TestEnv::new("listtextfull");
    env.run_ok(&[
        "add",
        "first line headline\nsecond line body content",
        "--type",
        "fact",
    ]);
    let out = env.run_ok(&["list", "--full"]);
    assert!(out.contains("first line headline"), "got: {out}");
    assert!(out.contains("second line body content"), "got: {out}");
}

#[test]
fn test_show_unchanged_has_content() {
    let env = TestEnv::new("showunchanged");
    let add = env.run_ok(&["add", "body text\nsecond line", "--type", "fact"]);
    let id = extract_id(&add);
    let out = env.run_ok(&["--json", "show", &id]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(parsed["unit"]["content"], "body text\nsecond line");
}

#[test]
fn test_headline_truncates_long_first_line() {
    let env = TestEnv::new("headlinetrunc");
    let long = "a".repeat(200);
    env.run_ok(&["add", &long, "--type", "fact"]);
    let out = env.run_ok(&["--json", "list"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON array");
    let headline = parsed[0]["headline"].as_str().unwrap();
    assert!(
        headline.ends_with("..."),
        "headline must end with ellipsis when truncated, got: {headline}"
    );
    // 120 chars + "..." = 123 char headline max
    let char_count = headline.chars().count();
    assert_eq!(
        char_count, 123,
        "truncated headline should be 120 chars + '...', got {char_count}: {headline}"
    );
}

#[test]
fn test_lean_includes_slug_when_bound() {
    let env = TestEnv::new("leanslug");
    let add = env.run_ok(&["add", "fact with slug", "--type", "fact"]);
    let id = extract_id(&add);
    env.run_ok(&["slug", "set", "my-fact", &id]);

    let out = env.run_ok(&["--json", "list"]);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).expect("valid JSON array");
    let row = parsed.iter().find(|u| u["id"] == id).expect("row present");
    assert_eq!(row["slug"], "my-fact");
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
    assert_eq!(parsed[0]["headline"], "original knowledge");
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

// ========================================================================
// scan --unstructured (frontmatter-p2, task fsuh)
// ========================================================================

/// Produce a body of at least 200 bytes so scan --unstructured considers it.
fn long_prose(seed: &str) -> String {
    let filler = "lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. ";
    format!("{seed} {filler}{filler}")
}

#[test]
fn test_scan_unstructured_empty_store() {
    let env = TestEnv::new("scan-unstruct-empty");
    let out = env.run_ok(&["scan", "--unstructured"]);
    assert!(out.contains("No unstructured units found"), "got: {out}");
}

#[test]
fn test_scan_unstructured_mixes_prose_and_frontmatter() {
    // UUIDv7 short_ids can collide for units added in the same ms, so match
    // on unique body keywords instead.
    let env = TestEnv::new("scan-unstruct-mix");
    let schema_file = env.dir.join("schema.md");
    std::fs::write(
        &schema_file,
        format!(
            "---\nname: schema-unit\n---\n{}\n",
            long_prose("SCHEMA_MARKER")
        ),
    )
    .unwrap();
    env.run_ok(&[
        "add",
        "--type",
        "aspect",
        "--from-file",
        schema_file.to_str().unwrap(),
    ]);

    env.run_ok(&["add", &long_prose("PROSE_MARKER"), "--type", "aspect"]);

    let out = env.run_ok(&["scan", "--unstructured"]);
    assert!(out.contains("PROSE_MARKER"), "prose aspect listed: {out}");
    assert!(
        !out.contains("SCHEMA_MARKER"),
        "schema'd aspect absent: {out}"
    );
}

#[test]
fn test_scan_unstructured_skips_short_bodies() {
    let env = TestEnv::new("scan-unstruct-short");
    // Body well under 200 B.
    let short_out = env.run_ok(&["add", "too short to rewrite", "--type", "fact"]);
    let short_id_ = extract_id(&short_out);
    let out = env.run_ok(&["scan", "--unstructured"]);
    assert!(
        !out.contains(&short_id(&short_id_).to_string()),
        "short body skipped: {out}"
    );
}

#[test]
fn test_scan_unstructured_sorts_aspect_first() {
    let env = TestEnv::new("scan-unstruct-aspect-first");
    // Procedure with many marks — still ranks below aspect with zero marks.
    let proc_out = env.run_ok(&["add", &long_prose("PROC_MARKER"), "--type", "procedure"]);
    let proc_id = extract_id(&proc_out);
    for _ in 0..5 {
        env.run_ok(&["mark", &proc_id, "--kind", "used"]);
    }

    env.run_ok(&["add", &long_prose("ASPECT_MARKER"), "--type", "aspect"]);

    let out = env.run_ok(&["scan", "--unstructured"]);
    let aspect_pos = out.find("ASPECT_MARKER").expect("aspect in output");
    let proc_pos = out.find("PROC_MARKER").expect("procedure in output");
    assert!(
        aspect_pos < proc_pos,
        "aspect must sort before procedure: {out}"
    );
}

#[test]
fn test_scan_unstructured_sorts_by_mark_count_then_confidence() {
    let env = TestEnv::new("scan-unstruct-marks-conf");
    let low_out = env.run_ok(&["add", &long_prose("LOW_MARKER"), "--type", "procedure"]);
    let low_id = extract_id(&low_out);
    let high_out = env.run_ok(&["add", &long_prose("HIGH_MARKER"), "--type", "procedure"]);
    let high_id = extract_id(&high_out);
    env.run_ok(&["mark", &low_id, "--kind", "used"]);
    for _ in 0..3 {
        env.run_ok(&["mark", &high_id, "--kind", "used"]);
    }

    let out = env.run_ok(&["scan", "--unstructured"]);
    let high_pos = out.find("HIGH_MARKER").expect("high in output");
    let low_pos = out.find("LOW_MARKER").expect("low in output");
    assert!(high_pos < low_pos, "higher mark count sorts first: {out}");
}

#[test]
fn test_scan_unstructured_type_filter_narrows() {
    let env = TestEnv::new("scan-unstruct-type-filter");
    env.run_ok(&["add", &long_prose("ASPECT_TF_MARKER"), "--type", "aspect"]);
    env.run_ok(&["add", &long_prose("PROC_TF_MARKER"), "--type", "procedure"]);

    let out = env.run_ok(&["scan", "--unstructured", "--type", "aspect"]);
    assert!(out.contains("ASPECT_TF_MARKER"), "aspect present: {out}");
    assert!(
        !out.contains("PROC_TF_MARKER"),
        "procedure filtered out: {out}"
    );
}

#[test]
fn test_scan_unstructured_json_shape() {
    let env = TestEnv::new("scan-unstruct-json");
    let out_a = env.run_ok(&["add", &long_prose("prose a"), "--type", "fact"]);
    let id_a = extract_id(&out_a);

    let out = env.run_ok(&["--json", "scan", "--unstructured"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    let arr = parsed.as_array().expect("top-level array");
    assert!(!arr.is_empty(), "non-empty: {out}");
    let first = &arr[0];
    assert!(first["id"].as_str().is_some(), "id present: {out}");
    assert!(first["type"].as_str().is_some(), "type present: {out}");
    assert!(first["slugs"].is_array(), "slugs array: {out}");
    assert!(first["marks"].is_number(), "marks number: {out}");
    assert!(first["confidence"].is_number(), "conf number: {out}");
    assert!(
        first["first_line"].as_str().is_some(),
        "first_line present: {out}"
    );
    assert!(
        arr.iter().any(|r| r["id"].as_str() == Some(&*id_a)),
        "target unit present: {out}"
    );
}

#[test]
fn test_scan_unstructured_excludes_superseded_by_default() {
    // F14: units with an incoming `supersedes` edge are already obsolete;
    // scan --unstructured must drop them from rewrite-priority ranking.
    let env = TestEnv::new("scan-unstruct-f14-default");
    let old_out = env.run_ok(&["add", &long_prose("OLD_VERSION"), "--type", "fact"]);
    let old_id = extract_id(&old_out);
    let new_out = env.run_ok(&["add", &long_prose("NEW_VERSION"), "--type", "fact"]);
    let new_id = extract_id(&new_out);
    // new supersedes old
    env.run_ok(&["link", &new_id, &old_id, "--rel", "supersedes"]);

    let out = env.run_ok(&["scan", "--unstructured"]);
    assert!(
        !out.contains("OLD_VERSION"),
        "superseded unit must be hidden by default: {out}"
    );
    assert!(out.contains("NEW_VERSION"), "current unit surfaces: {out}");
}

#[test]
fn test_scan_unstructured_include_superseded_opts_in() {
    // F14: --include-superseded brings the obsolete units back for audits.
    let env = TestEnv::new("scan-unstruct-f14-optin");
    let old_out = env.run_ok(&["add", &long_prose("OLD_AUDIT"), "--type", "fact"]);
    let old_id = extract_id(&old_out);
    let new_out = env.run_ok(&["add", &long_prose("NEW_AUDIT"), "--type", "fact"]);
    let new_id = extract_id(&new_out);
    env.run_ok(&["link", &new_id, &old_id, "--rel", "supersedes"]);

    let out = env.run_ok(&["scan", "--unstructured", "--include-superseded"]);
    assert!(
        out.contains("OLD_AUDIT"),
        "superseded unit surfaces with opt-in: {out}"
    );
    assert!(
        out.contains("NEW_AUDIT"),
        "current unit also surfaces: {out}"
    );
}

/// Write `body` to a temp file under `env.dir` and return its path.
/// Helper for tests that add units with leading `---` frontmatter (clap
/// rejects leading-dash positional args; `--from-file` bypasses this).
fn write_body(env: &TestEnv, name: &str, body: &str) -> String {
    let path = env.dir.join(name);
    std::fs::write(&path, body).expect("write body fixture");
    path.to_string_lossy().to_string()
}

#[test]
fn test_sync_refs_creates_related_to_on_add() {
    // F15: frontmatter `refs:` entries materialize as related_to edges.
    let env = TestEnv::new("sync-refs-add");
    // Target unit has a slug we can reference.
    let tgt = env.run_ok(&["add", "rust cli target", "--type", "fact"]);
    let tgt_id = extract_id(&tgt);
    env.run_ok(&["slug", "set", "ref-target-slug", &tgt_id]);

    // Add a unit with frontmatter listing the target slug under refs.
    let body = "---\nscope: linking test\nrefs:\n  - ref-target-slug\n---\n\nbody text";
    let path = write_body(&env, "refs-add.md", body);
    let out = env.run_ok(&["add", "--type", "fact", "--from-file", &path]);
    let src_id = extract_id(&out);

    // show should list an outgoing related_to edge to the target.
    let shown = env.run_ok(&["show", &src_id]);
    assert!(
        shown.contains(&format!("-> {tgt_id} (related_to)")),
        "expected edge to target: {shown}"
    );
}

#[test]
fn test_sync_refs_idempotent_on_re_edit() {
    // F15: re-edits must not duplicate edges (INSERT OR IGNORE).
    let env = TestEnv::new("sync-refs-idempotent");
    let tgt = env.run_ok(&["add", "target body", "--type", "fact"]);
    let tgt_id = extract_id(&tgt);
    env.run_ok(&["slug", "set", "target-s", &tgt_id]);

    let body = "---\nscope: test\nrefs:\n  - target-s\n---\n\nbody";
    let path = write_body(&env, "refs-idem.md", body);
    let out = env.run_ok(&["add", "--type", "fact", "--from-file", &path]);
    let src_id = extract_id(&out);

    // Re-edit same content via --from-file (triggers update_unit path).
    env.run_ok(&[
        "edit",
        &src_id,
        "--from-file",
        &path,
        "--source",
        "second-pass",
    ]);

    // Assert exactly one outgoing related_to edge src→tgt.
    let json = env.run_ok(&["show", &src_id, "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    let outgoing = parsed["links"]["outgoing"]
        .as_array()
        .expect("outgoing array");
    let related_to_tgt: Vec<_> = outgoing
        .iter()
        .filter(|l| {
            l["relationship"].as_str() == Some("related_to") && l["to_id"].as_str() == Some(&tgt_id)
        })
        .collect();
    assert_eq!(
        related_to_tgt.len(),
        1,
        "expected exactly one related_to edge after re-edit, got: {related_to_tgt:?}"
    );
}

#[test]
fn test_sync_refs_warns_on_unknown_target() {
    // F15: unresolvable ref → stderr warn + skip, not fail.
    let env = TestEnv::new("sync-refs-unknown");
    let body = "---\nscope: test\nrefs:\n  - ghost-slug-does-not-exist\n---\n\nbody";
    let path = write_body(&env, "refs-ghost.md", body);
    let output = env.run(&["add", "--type", "fact", "--from-file", &path]);
    assert!(output.status.success(), "add must succeed despite bad ref");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("does not resolve") || stderr.contains("ghost-slug"),
        "expected warn about unresolvable ref, got stderr: {stderr}"
    );
}

#[test]
fn test_sync_refs_accepts_uuid_and_slug_with_hint() {
    // F15: refs may be bare UUID, slug, or `slug (uuid)` combo (the form
    // the dogfood-log uses). First-whitespace-token is treated as the
    // resolvable token.
    let env = TestEnv::new("sync-refs-formats");
    let t1 = env.run_ok(&["add", "t1 body", "--type", "fact"]);
    let t1_id = extract_id(&t1);
    env.run_ok(&["slug", "set", "t1-slug", &t1_id]);

    let body =
        format!("---\nscope: test\nrefs:\n  - t1-slug (019d-hint)\n  - {t1_id}\n---\n\nbody");
    let path = write_body(&env, "refs-formats.md", &body);
    let out = env.run_ok(&["add", "--type", "fact", "--from-file", &path]);
    let src_id = extract_id(&out);

    let json = env.run_ok(&["show", &src_id, "--json"]);
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
    let outgoing = parsed["links"]["outgoing"]
        .as_array()
        .expect("outgoing array");
    // Both refs point at t1 — expect exactly 1 edge (slug + UUID resolve to
    // the same target and INSERT OR IGNORE dedups).
    let hits: Vec<_> = outgoing
        .iter()
        .filter(|l| {
            l["relationship"].as_str() == Some("related_to") && l["to_id"].as_str() == Some(&t1_id)
        })
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one edge to t1 from two aliased refs: {hits:?}"
    );
}

#[test]
fn test_scan_unstructured_unchanged_when_flag_absent() {
    // Regression guard — plain `scan` still produces the standard health
    // report, not the unstructured list, even when unstructured candidates
    // exist.
    let env = TestEnv::new("scan-unstruct-regress");
    env.run_ok(&["add", &long_prose("bare prose"), "--type", "fact"]);
    let out = env.run_ok(&["scan"]);
    assert!(
        !out.contains("first-line"),
        "standard scan does not print unstructured header: {out}"
    );
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
    assert_eq!(version, 4, "expected user_version=4, got {version}");

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
    assert_eq!(version, 4);

    let slugs_present: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='slugs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(slugs_present, 1);

    // Two-step migration: v2→v3 added slugs, v3→v4 added archived. Both
    // must have run and the seeded row must survive the column add.
    let archived_present: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('units') WHERE name='archived'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(archived_present, 1);
    let seeded_archived: i64 = conn
        .query_row(
            "SELECT archived FROM units WHERE id='0193-seed-unit-aaaa-aaaaaaaaaaaa'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(seeded_archived, 0, "seeded row must default to archived=0");

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

    let out = env.run_ok(&[
        "--json",
        "emit",
        "--target",
        "claude-code",
        "--type",
        "aspect",
    ]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(parsed["written"], serde_json::json!(["jsonner"]));
    assert_eq!(parsed["swept"], serde_json::json!([]));
    assert_eq!(parsed["skipped_uuids"], serde_json::json!([]));
    assert!(parsed["target_dir"].is_string(), "target_dir string");
}

// ---------------------------------------------------------------------------
// size_guard (Story 3 — write-time size signal)
// ---------------------------------------------------------------------------

// Thresholds for tests are passed via SIMARIS_WARN_BYTES / SIMARIS_HARD_BYTES
// so bodies stay small while still crossing limits.

#[test]
fn test_add_size_warn_stderr_cites_slug() {
    let env = TestEnv::new("size-warn");
    let body = "x".repeat(80);
    let output = env.run_with_env(
        &["add", &body, "--type", "fact"],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "200")],
    );
    assert!(
        output.status.success(),
        "warn must not reject: {:?}",
        output
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("split-ruleset"),
        "stderr lacks citation: {stderr}"
    );
    assert!(stderr.contains("80"), "stderr lacks actual bytes: {stderr}");
    assert!(stderr.contains("50"), "stderr lacks warn bytes: {stderr}");
    assert!(
        stderr.contains("warn threshold"),
        "stderr lacks warn label: {stderr}"
    );
}

#[test]
fn test_add_size_hard_rejects_without_force() {
    let env = TestEnv::new("size-hard");
    let body = "x".repeat(150);
    let output = env.run_with_env(
        &["add", &body, "--type", "fact"],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "100")],
    );
    assert!(!output.status.success(), "hard must reject: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("split-ruleset"),
        "stderr lacks citation: {stderr}"
    );
    assert!(
        stderr.contains("150"),
        "stderr lacks actual bytes: {stderr}"
    );
    assert!(stderr.contains("100"), "stderr lacks hard bytes: {stderr}");
    assert!(
        stderr.contains("hard threshold"),
        "stderr lacks hard label: {stderr}"
    );

    // DB should be empty — reject happened before insert.
    let list = env.run_ok(&["list"]);
    assert!(!list.contains(" fact "), "unit must not land: {list}");
}

#[test]
fn test_add_size_force_overrides_hard() {
    let env = TestEnv::new("size-force");
    let body = "x".repeat(150);
    let output = env.run_with_env(
        &["add", &body, "--type", "fact", "--force"],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "100")],
    );
    assert!(
        output.status.success(),
        "--force must succeed: {:?}",
        output
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("split-ruleset"),
        "force still warns: {stderr}"
    );
    assert!(
        stderr.contains("hard threshold"),
        "force cites hard: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("Added unit "), "unit lands: {stdout}");
}

#[test]
fn test_add_size_flow_tag_silent() {
    let env = TestEnv::new("size-flow-tag");
    let body = "x".repeat(150);
    let output = env.run_with_env(
        &["add", &body, "--type", "procedure", "--tags", "flow"],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "100")],
    );
    assert!(output.status.success(), "flow tag bypasses: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("split-ruleset"),
        "flow tag must be silent: {stderr}"
    );
}

#[test]
fn test_add_size_flow_flag_silent() {
    let env = TestEnv::new("size-flow-flag");
    let body = "x".repeat(150);
    let output = env.run_with_env(
        &["add", &body, "--type", "procedure", "--flow"],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "100")],
    );
    assert!(output.status.success(), "--flow bypasses: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("split-ruleset"),
        "--flow must be silent: {stderr}"
    );
}

#[test]
fn test_edit_size_hard_rejects_without_force() {
    let env = TestEnv::new("size-edit-hard");
    // First insert a small unit under thresholds that pass defaults.
    let add_out = env.run_ok(&["add", "tiny", "--type", "fact"]);
    let id = extract_id(&add_out);

    // Now edit with a too-big content under tight thresholds.
    let big = "x".repeat(150);
    let output = env.run_with_env(
        &["edit", &id, "--content", &big],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "100")],
    );
    assert!(!output.status.success(), "edit hard rejects: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("hard threshold"), "cites hard: {stderr}");

    // Content unchanged — original body still present.
    let show = env.run_ok(&["show", &id]);
    assert!(show.contains("tiny"), "original content preserved: {show}");
}

#[test]
fn test_edit_size_force_allows_large_content() {
    let env = TestEnv::new("size-edit-force");
    let add_out = env.run_ok(&["add", "tiny", "--type", "fact"]);
    let id = extract_id(&add_out);

    let big = "y".repeat(150);
    let output = env.run_with_env(
        &["edit", &id, "--content", &big, "--force"],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "100")],
    );
    assert!(output.status.success(), "--force allows edit: {:?}", output);
    let show = env.run_ok(&["show", &id]);
    assert!(show.contains(&big), "new content stored");
}

#[test]
fn test_edit_size_no_content_change_never_flags() {
    // Retroactive-safety: editing a tag on an already-large existing unit
    // must not trigger size check. We seed the unit via --force, then edit
    // only its tags under tight thresholds.
    let env = TestEnv::new("size-edit-tagonly");
    let big = "z".repeat(150);
    let add = env.run_with_env(
        &["add", &big, "--type", "fact", "--force"],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "100")],
    );
    assert!(add.status.success());
    let id = extract_id(&String::from_utf8_lossy(&add.stdout));

    let edit_out = env.run_with_env(
        &["edit", &id, "--tags", "flow"],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "100")],
    );
    assert!(
        edit_out.status.success(),
        "tag-only edit must not reject: {:?}",
        edit_out
    );
    let stderr = String::from_utf8_lossy(&edit_out.stderr);
    assert!(
        !stderr.contains("hard threshold"),
        "tag-only edit must not flag: {stderr}"
    );
}

#[test]
fn test_show_list_never_flag_existing_oversize_units() {
    // No retroactive enforcement — units stored above threshold must not
    // trigger any size warning on read paths.
    let env = TestEnv::new("size-no-retro");
    let big = "w".repeat(150);
    let add = env.run_with_env(
        &["add", &big, "--type", "fact", "--force"],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "100")],
    );
    assert!(add.status.success());
    let id = extract_id(&String::from_utf8_lossy(&add.stdout));

    let show = env.run_with_env(
        &["show", &id],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "100")],
    );
    assert!(show.status.success());
    let show_stderr = String::from_utf8_lossy(&show.stderr);
    assert!(
        !show_stderr.contains("split-ruleset"),
        "show must not warn: {show_stderr}"
    );

    let list = env.run_with_env(
        &["list"],
        &[("SIMARIS_WARN_BYTES", "50"), ("SIMARIS_HARD_BYTES", "100")],
    );
    let list_stderr = String::from_utf8_lossy(&list.stderr);
    assert!(
        !list_stderr.contains("split-ruleset"),
        "list must not warn: {list_stderr}"
    );
}

#[test]
fn test_add_defaults_allow_small_bodies_silently() {
    // Default thresholds (2048/8192) must leave normal-sized adds silent.
    let env = TestEnv::new("size-default-silent");
    let output = env.run(&["add", "normal sized content", "--type", "fact"]);
    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("split-ruleset"),
        "default thresholds must be silent: {stderr}"
    );
}

#[test]
fn test_size_env_var_bad_value_falls_back_to_default() {
    // Non-numeric env var must not crash — fall back to default thresholds.
    let env = TestEnv::new("size-bad-env");
    let output = env.run_with_env(
        &["add", "tiny", "--type", "fact"],
        &[
            ("SIMARIS_WARN_BYTES", "not-a-number"),
            ("SIMARIS_HARD_BYTES", "also-bad"),
        ],
    );
    assert!(
        output.status.success(),
        "bad env should fall back silently: {:?}",
        output
    );
}

// --- Frontmatter (P0) ---------------------------------------------------

#[test]
fn test_show_frontmatter_roundtrip() {
    let env = TestEnv::new("fm-roundtrip");
    let body = "---\ntitle: hello\nstatus: draft\n---\nbody prose here\n";
    let out = env.run_ok(&["add", "--type", "fact", "--", body]);
    let id = extract_id(&out);

    let shown = env.run_ok(&["show", &id]);
    assert!(
        shown.contains("**title:** hello"),
        "missing title field line: {shown}"
    );
    assert!(
        shown.contains("**status:** draft"),
        "missing status field line: {shown}"
    );
    assert!(
        shown.contains("body prose here"),
        "missing body text: {shown}"
    );
    // Rendered view must not echo raw fences.
    assert!(
        !shown.contains("---\ntitle:"),
        "rendered view leaked raw fences: {shown}"
    );
}

#[test]
fn test_show_no_frontmatter_unchanged() {
    let env = TestEnv::new("fm-none");
    let body = "plain prose no fences";
    let out = env.run_ok(&["add", body, "--type", "fact"]);
    let id = extract_id(&out);

    let shown = env.run_ok(&["show", &id]);
    assert!(shown.contains(body), "body missing: {shown}");
    assert!(
        !shown.contains("**"),
        "unexpected markdown field markers: {shown}"
    );
}

#[test]
fn test_show_malformed_frontmatter_falls_back() {
    let env = TestEnv::new("fm-malformed");
    // Invalid YAML inside fences — must not crash, must render body.
    let body = "---\n: : bad yaml :: :\n---\nfallback body\n";
    let out = env.run_ok(&["add", "--type", "fact", "--", body]);
    let id = extract_id(&out);

    let output = env.run(&["show", &id]);
    assert!(
        output.status.success(),
        "show must not crash on malformed yaml: {:?}",
        output
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Body present, no field lines rendered.
    assert!(stdout.contains("fallback body"), "body missing: {stdout}");
    assert!(
        !stdout.contains("**title:"),
        "unexpected rendered field: {stdout}"
    );
}

#[test]
fn test_show_raw_prints_fences_verbatim() {
    let env = TestEnv::new("fm-raw");
    let body = "---\ntitle: hello\n---\nbody text\n";
    let out = env.run_ok(&["add", "--type", "fact", "--", body]);
    let id = extract_id(&out);

    let shown = env.run_ok(&["show", &id, "--raw"]);
    assert!(
        shown.contains("---\ntitle: hello\n---"),
        "raw output missing literal fences: {shown}"
    );
    // Raw mode must not render parsed markdown field lines.
    assert!(
        !shown.contains("**title:**"),
        "raw mode leaked rendered field: {shown}"
    );
}

#[test]
fn test_show_json_content_has_raw_fences() {
    let env = TestEnv::new("fm-json");
    let body = "---\ntitle: hello\n---\nbody text\n";
    let out = env.run_ok(&["add", "--type", "fact", "--", body]);
    let id = extract_id(&out);

    let shown = env.run_ok(&["show", &id, "--json"]);
    let v: serde_json::Value = serde_json::from_str(&shown).expect("json parse");
    let content = v
        .get("unit")
        .and_then(|u| u.get("content"))
        .and_then(|c| c.as_str())
        .expect("content field");
    assert!(
        content.contains("---\ntitle: hello\n---"),
        "json content missing raw fences: {content}"
    );
}

// --- Frontmatter (P1 write-path flags) ---------------------------------

#[test]
fn test_add_procedure_all_flags_renders_fields() {
    let env = TestEnv::new("fm-p1-procedure-all");
    let out = env.run_ok(&[
        "add",
        "body prose",
        "--type",
        "procedure",
        "--trigger",
        "weekly, after digest",
        "--check",
        "report on maintenance-log",
        "--cadence",
        "monthly full",
        "--prereq",
        "@simaris-maint",
        "--ref",
        "@frontmatter-proposal",
    ]);
    let id = extract_id(&out);

    let shown = env.run_ok(&["show", &id]);
    assert!(
        shown.contains("**trigger:** weekly, after digest"),
        "trigger line: {shown}"
    );
    assert!(
        shown.contains("**check:** report on maintenance-log"),
        "check line: {shown}"
    );
    assert!(
        shown.contains("**cadence:** monthly full"),
        "cadence line: {shown}"
    );
    assert!(shown.contains("body prose"), "body missing: {shown}");
    assert!(
        !shown.contains("---\ntrigger:"),
        "rendered view leaked raw fences: {shown}"
    );
}

#[test]
fn test_add_aspect_repeated_dispatches_to_becomes_list() {
    let env = TestEnv::new("fm-p1-aspect-dispatches");
    let out = env.run_ok(&[
        "add",
        "lotus routes prompts",
        "--type",
        "aspect",
        "--role",
        "front-door router",
        "--dispatches-to",
        "researcher",
        "--dispatches-to",
        "project-manager",
    ]);
    let id = extract_id(&out);

    let shown = env.run_ok(&["show", &id]);
    assert!(
        shown.contains("**role:** front-door router"),
        "role line: {shown}"
    );
    // short list ≤3 items renders inline per P0 markdown renderer
    assert!(
        shown.contains("**dispatches_to:** researcher, project-manager"),
        "dispatches_to inline list: {shown}"
    );

    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(
        raw.contains("dispatches_to:")
            && raw.contains("- researcher")
            && raw.contains("- project-manager"),
        "raw yaml sequence: {raw}"
    );
}

#[test]
fn test_add_fact_evidence_renders() {
    let env = TestEnv::new("fm-p1-fact-evidence");
    let out = env.run_ok(&[
        "add",
        "rust uses ownership",
        "--type",
        "fact",
        "--scope",
        "memory management",
        "--evidence",
        "rust-lang.org/book ch4",
    ]);
    let id = extract_id(&out);

    let shown = env.run_ok(&["show", &id]);
    assert!(
        shown.contains("**scope:** memory management"),
        "scope line: {shown}"
    );
    assert!(
        shown.contains("**evidence:** rust-lang.org/book ch4"),
        "evidence line: {shown}"
    );
}

#[test]
fn test_add_flag_on_wrong_type_errors_with_useful_message() {
    let env = TestEnv::new("fm-p1-wrong-type");
    let output = env.run(&["add", "body", "--type", "fact", "--trigger", "weekly"]);
    assert!(!output.status.success(), "must fail: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--trigger"), "err msg names flag: {stderr}");
    assert!(
        stderr.contains("procedure"),
        "err msg cites valid type: {stderr}"
    );
    assert!(stderr.contains("fact"), "err msg cites bad type: {stderr}");
}

#[test]
fn test_add_from_file_with_valid_frontmatter_stored_verbatim() {
    let env = TestEnv::new("fm-p1-from-file-ok");
    let file = env.dir.join("draft.md");
    let body = "---\ntrigger: on demand\ncheck: passes\n---\nbody from file\n";
    std::fs::write(&file, body).unwrap();

    let out = env.run_ok(&[
        "add",
        "--type",
        "procedure",
        "--from-file",
        file.to_str().unwrap(),
    ]);
    let id = extract_id(&out);

    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(
        raw.contains("---\ntrigger: on demand\ncheck: passes\n---\nbody from file"),
        "stored verbatim: {raw}"
    );
}

#[test]
fn test_add_from_file_plus_field_flag_mutex_error() {
    let env = TestEnv::new("fm-p1-mutex");
    let file = env.dir.join("draft.md");
    std::fs::write(&file, "---\ntrigger: x\n---\nbody\n").unwrap();

    let output = env.run(&[
        "add",
        "--type",
        "procedure",
        "--from-file",
        file.to_str().unwrap(),
        "--trigger",
        "also here",
    ]);
    assert!(!output.status.success(), "must fail: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mutually exclusive"),
        "err cites mutex: {stderr}"
    );
    assert!(stderr.contains("--from-file"), "err names flag: {stderr}");
}

#[test]
fn test_add_from_file_malformed_yaml_errors_cleanly() {
    let env = TestEnv::new("fm-p1-from-file-bad");
    let file = env.dir.join("bad.md");
    std::fs::write(&file, "---\n: : bad yaml :: :\n---\nbody\n").unwrap();

    let output = env.run(&[
        "add",
        "--type",
        "procedure",
        "--from-file",
        file.to_str().unwrap(),
    ]);
    assert!(!output.status.success(), "must reject malformed yaml");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("malformed frontmatter"),
        "err cites malformed: {stderr}"
    );
    // Must be an error, not a Rust panic.
    assert!(!stderr.contains("panicked at"), "must not panic: {stderr}");
}

#[test]
fn test_add_no_flags_no_from_file_unchanged_regression_guard() {
    let env = TestEnv::new("fm-p1-regression");
    let out = env.run_ok(&["add", "plain body", "--type", "fact"]);
    let id = extract_id(&out);

    let shown = env.run_ok(&["show", &id]);
    assert!(shown.contains("plain body"), "body preserved: {shown}");
    // No frontmatter fences should be injected when no flags used.
    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(
        !raw.contains("---\n"),
        "raw content must have no injected frontmatter: {raw}"
    );
}

// --- Frontmatter (P1.5 edit-merge + clobber guard) ---------------------

/// Seed a schema'd procedure unit via `add --trigger ... --check ...` and
/// return its id. Shared by several P1.5 edit tests.
fn seed_procedure_with_frontmatter(env: &TestEnv) -> String {
    let out = env.run_ok(&[
        "add",
        "original body",
        "--type",
        "procedure",
        "--trigger",
        "weekly",
        "--check",
        "report ok",
        "--cadence",
        "monthly",
    ]);
    extract_id(&out)
}

#[test]
fn test_edit_content_on_frontmatter_unit_preserves_fields() {
    let env = TestEnv::new("fm-p1-5-edit-content-preserves");
    let id = seed_procedure_with_frontmatter(&env);

    env.run_ok(&["edit", &id, "--content", "new body text"]);

    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(
        raw.contains("trigger: weekly"),
        "trigger preserved in raw: {raw}"
    );
    assert!(
        raw.contains("check: report ok"),
        "check preserved in raw: {raw}"
    );
    assert!(
        raw.contains("cadence: monthly"),
        "cadence preserved in raw: {raw}"
    );
    assert!(raw.contains("new body text"), "body swapped in raw: {raw}");
    assert!(
        !raw.contains("original body"),
        "old body gone from raw: {raw}"
    );
}

#[test]
fn test_edit_field_flag_updates_only_that_field() {
    let env = TestEnv::new("fm-p1-5-edit-field-update");
    let id = seed_procedure_with_frontmatter(&env);

    env.run_ok(&["edit", &id, "--trigger", "daily, after lunch"]);

    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(
        raw.contains("trigger: daily, after lunch"),
        "trigger updated: {raw}"
    );
    assert!(raw.contains("check: report ok"), "check untouched: {raw}");
    assert!(raw.contains("cadence: monthly"), "cadence untouched: {raw}");
    assert!(
        raw.contains("original body"),
        "body untouched when only field flag set: {raw}"
    );
}

#[test]
fn test_edit_field_flag_on_unit_without_frontmatter_adds_block() {
    let env = TestEnv::new("fm-p1-5-edit-adds-fm");
    // Seed a plain procedure (no frontmatter injected).
    let out = env.run_ok(&["add", "plain procedure body", "--type", "procedure"]);
    let id = extract_id(&out);

    // Confirm no fences initially.
    let raw_before = env.run_ok(&["show", &id, "--raw"]);
    assert!(
        !raw_before.contains("---\n"),
        "no fm before edit: {raw_before}"
    );

    env.run_ok(&["edit", &id, "--trigger", "fresh"]);

    let raw_after = env.run_ok(&["show", &id, "--raw"]);
    // Body of raw output includes the `---\n` fence on its own line.
    assert!(
        raw_after.contains("\n---\ntrigger: fresh\n---\n"),
        "frontmatter block inserted: {raw_after}"
    );
    assert!(
        raw_after.contains("plain procedure body"),
        "body preserved: {raw_after}"
    );
}

#[test]
fn test_edit_replace_all_clobbers_frontmatter() {
    let env = TestEnv::new("fm-p1-5-edit-replace-all");
    let id = seed_procedure_with_frontmatter(&env);

    env.run_ok(&[
        "edit",
        &id,
        "--replace-all",
        "--content",
        "total clobber body",
    ]);

    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(
        !raw.contains("---\n"),
        "frontmatter gone after --replace-all: {raw}"
    );
    assert!(!raw.contains("trigger:"), "no trigger key left: {raw}");
    assert!(
        raw.contains("total clobber body"),
        "new body present: {raw}"
    );
}

#[test]
fn test_edit_field_flag_on_wrong_type_errors() {
    let env = TestEnv::new("fm-p1-5-edit-wrong-type");
    // Seed a plain fact — --trigger does not apply to facts.
    let out = env.run_ok(&["add", "a fact body", "--type", "fact"]);
    let id = extract_id(&out);

    let output = env.run(&["edit", &id, "--trigger", "weekly"]);
    assert!(!output.status.success(), "must fail: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--trigger"), "err names flag: {stderr}");
    assert!(
        stderr.contains("procedure"),
        "err cites valid type: {stderr}"
    );
    assert!(stderr.contains("fact"), "err cites stored type: {stderr}");
}

#[test]
fn test_edit_field_flag_plus_content_merges_both() {
    let env = TestEnv::new("fm-p1-5-edit-field-and-content");
    let id = seed_procedure_with_frontmatter(&env);

    env.run_ok(&[
        "edit",
        &id,
        "--trigger",
        "hourly",
        "--content",
        "fresh body",
    ]);

    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(raw.contains("trigger: hourly"), "trigger updated: {raw}");
    assert!(raw.contains("check: report ok"), "other field kept: {raw}");
    assert!(raw.contains("fresh body"), "body swapped: {raw}");
    assert!(!raw.contains("original body"), "old body gone: {raw}");
}

#[test]
fn test_edit_from_file_plus_field_flag_mutex_error() {
    let env = TestEnv::new("fm-p1-5-edit-from-file-mutex");
    let id = seed_procedure_with_frontmatter(&env);

    let file = env.dir.join("draft.md");
    std::fs::write(&file, "---\ntrigger: from file\n---\nbody from file\n").unwrap();

    let output = env.run(&[
        "edit",
        &id,
        "--from-file",
        file.to_str().unwrap(),
        "--trigger",
        "also here",
    ]);
    assert!(!output.status.success(), "must fail: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("mutually exclusive"),
        "err cites mutex: {stderr}"
    );
    assert!(stderr.contains("--from-file"), "err names flag: {stderr}");
}

#[test]
fn test_edit_prose_unit_content_only_unchanged_regression_guard() {
    let env = TestEnv::new("fm-p1-5-edit-prose-regression");
    // Plain idea unit — no schema at all.
    let out = env.run_ok(&["add", "original idea text", "--type", "idea"]);
    let id = extract_id(&out);

    env.run_ok(&["edit", &id, "--content", "updated idea text"]);

    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(
        !raw.contains("---\n"),
        "no fm injected on prose edit: {raw}"
    );
    assert!(raw.contains("updated idea text"), "new body present: {raw}");
    assert!(!raw.contains("original idea text"), "old body gone: {raw}");

    // Inspect JSON to confirm exact content (no trailing metadata noise).
    let json = env.run_ok(&["show", &id, "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let content = v["unit"]["content"].as_str().expect("content field");
    assert_eq!(content, "updated idea text", "content exact match");
}

// --- P3a rewrite (editor core) -----------------------------------------

/// Return a `SIMARIS_EDITOR` shell command that overwrites its `$1` argument
/// (the temp file path) with the contents of `fixture_path`. Matches the
/// editor contract: we feed `sh -c "<cmd> <temp_path>"`, so `$0` inside the
/// inner script receives the temp path (since `sh -c '<cmd>' <arg>` sets
/// `$0` to `<arg>`). We use that trick so the test fixture reliably
/// replaces the buffer.
fn editor_replaces_with(fixture_path: &std::path::Path) -> String {
    // The spawned command is: sh -c "<editor_cmd> <temp_path>".
    // Putting the editor command in quotes above, we need the inner script
    // to cat the fixture into the temp path. The temp path arrives as the
    // last positional arg — we read it via "$1" inside another sh -c.
    format!("sh -c 'cat \"{}\" > \"$1\"' --", fixture_path.display())
}

fn editor_noop() -> String {
    // Touch the file without changing it.
    "true".to_string()
}

fn editor_empties() -> String {
    "sh -c ': > \"$1\"' --".to_string()
}

#[test]
fn test_rewrite_prose_to_structured() {
    let env = TestEnv::new("fm-p3a-prose-to-structured");
    let out = env.run_ok(&["add", "original procedure body", "--type", "procedure"]);
    let id = extract_id(&out);

    let fixture = env.dir.join("replacement.md");
    std::fs::write(
        &fixture,
        "---\ntrigger: weekly\ncheck: green\n---\nrewritten body\n",
    )
    .unwrap();

    env.run_with_env(
        &["rewrite", &id],
        &[("SIMARIS_EDITOR", &editor_replaces_with(&fixture))],
    );

    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(raw.contains("trigger: weekly"), "fm written: {raw}");
    assert!(raw.contains("rewritten body"), "body written: {raw}");
}

#[test]
fn test_rewrite_structured_noop_leaves_unit() {
    let env = TestEnv::new("fm-p3a-noop");
    let fixture = env.dir.join("seed.md");
    std::fs::write(&fixture, "---\ntrigger: x\n---\nbody\n").unwrap();
    let out = env.run_ok(&[
        "add",
        "--type",
        "procedure",
        "--from-file",
        fixture.to_str().unwrap(),
    ]);
    let id = extract_id(&out);
    let before = env.run_ok(&["show", &id, "--raw"]);

    let output = env.run_with_env(&["rewrite", &id], &[("SIMARIS_EDITOR", &editor_noop())]);
    assert!(output.status.success(), "noop exits 0: {:?}", output);

    let after = env.run_ok(&["show", &id, "--raw"]);
    assert_eq!(before, after, "content unchanged on noop");
}

#[test]
fn test_rewrite_empty_buffer_aborts() {
    let env = TestEnv::new("fm-p3a-empty-abort");
    let out = env.run_ok(&["add", "keep this body", "--type", "fact"]);
    let id = extract_id(&out);
    let before = env.run_ok(&["show", &id, "--raw"]);

    let output = env.run_with_env(&["rewrite", &id], &[("SIMARIS_EDITOR", &editor_empties())]);
    assert!(output.status.success(), "abort exits 0: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("abort"), "stderr announces abort: {stderr}");

    let after = env.run_ok(&["show", &id, "--raw"]);
    assert_eq!(before, after, "content unchanged on abort");
}

#[test]
fn test_rewrite_invalid_yaml_rejected() {
    let env = TestEnv::new("fm-p3a-bad-yaml");
    let out = env.run_ok(&["add", "starting body", "--type", "procedure"]);
    let id = extract_id(&out);
    let before = env.run_ok(&["show", &id, "--raw"]);

    let fixture = env.dir.join("bad.md");
    std::fs::write(&fixture, "---\n: : bad yaml :: :\n---\nbody\n").unwrap();

    let output = env.run_with_env(
        &["rewrite", &id],
        &[("SIMARIS_EDITOR", &editor_replaces_with(&fixture))],
    );
    assert!(
        !output.status.success(),
        "invalid yaml rejects: {:?}",
        output
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("malformed") || stderr.contains("invalid frontmatter"),
        "err cites frontmatter: {stderr}"
    );

    let after = env.run_ok(&["show", &id, "--raw"]);
    assert_eq!(before, after, "content unchanged on reject");
}

#[test]
fn test_rewrite_template_only_skeleton() {
    let env = TestEnv::new("fm-p3a-template-only");
    let out = env.run_ok(&["add", "existing body text", "--type", "procedure"]);
    let id = extract_id(&out);

    // Editor dumps the seed buffer out so we can inspect composition.
    let capture = env.dir.join("captured.md");
    let editor_cmd = format!("sh -c 'cp \"$1\" \"{}\"' --", capture.display());
    env.run_with_env(
        &["rewrite", &id, "--template-only"],
        &[("SIMARIS_EDITOR", &editor_cmd)],
    );

    let seen = std::fs::read_to_string(&capture).unwrap();
    assert!(seen.contains("trigger:"), "skeleton fields present: {seen}");
    assert!(
        !seen.contains("existing body text"),
        "body excluded: {seen}"
    );
}

#[test]
fn test_rewrite_preserves_tags_and_slug() {
    let env = TestEnv::new("fm-p3a-preserve");
    let out = env.run_ok(&[
        "add",
        "tagged prose",
        "--type",
        "procedure",
        "--tags",
        "alpha,beta",
    ]);
    let id = extract_id(&out);
    env.run_ok(&["slug", "set", "my-proc", &id]);

    let fixture = env.dir.join("rewrite.md");
    std::fs::write(&fixture, "---\ntrigger: daily\n---\nnew body\n").unwrap();

    env.run_with_env(
        &["rewrite", &id],
        &[("SIMARIS_EDITOR", &editor_replaces_with(&fixture))],
    );

    let json = env.run_ok(&["show", &id, "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let tags = v["unit"]["tags"].as_array().expect("tags array");
    assert_eq!(tags.len(), 2, "tags preserved: {tags:?}");
    let slugs = v["slugs"].as_array().expect("slugs array");
    assert!(
        slugs.iter().any(|s| s.as_str() == Some("my-proc")),
        "slug preserved: {slugs:?}"
    );
}

#[test]
fn test_rewrite_nonexistent_id_clean_error() {
    let env = TestEnv::new("fm-p3a-missing");
    let output = env.run_with_env(
        &["rewrite", "019abcde-0000-7000-8000-000000000000"],
        &[("SIMARIS_EDITOR", "true")],
    );
    assert!(!output.status.success(), "missing id fails: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked at"), "no panic: {stderr}");
}

#[test]
fn test_rewrite_resolves_slug() {
    let env = TestEnv::new("fm-p3a-slug");
    let out = env.run_ok(&["add", "slugged prose", "--type", "fact"]);
    let id = extract_id(&out);
    env.run_ok(&["slug", "set", "my-fact", &id]);

    let fixture = env.dir.join("rw.md");
    std::fs::write(&fixture, "---\nscope: local\n---\nnew body\n").unwrap();

    let output = env.run_with_env(
        &["rewrite", "my-fact"],
        &[("SIMARIS_EDITOR", &editor_replaces_with(&fixture))],
    );
    assert!(output.status.success(), "slug resolves: {:?}", output);
    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(raw.contains("scope: local"), "rewrite applied: {raw}");
}

#[test]
fn test_rewrite_header_comments_stripped() {
    let env = TestEnv::new("fm-p3a-strip-comments");
    let out = env.run_ok(&["add", "prose body", "--type", "idea"]);
    let id = extract_id(&out);

    let fixture = env.dir.join("with-comments.md");
    std::fs::write(&fixture, "# user comment\n# another\n\nreal body\n").unwrap();

    env.run_with_env(
        &["rewrite", &id],
        &[("SIMARIS_EDITOR", &editor_replaces_with(&fixture))],
    );

    let json = env.run_ok(&["show", &id, "--json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let content = v["unit"]["content"].as_str().expect("content field");
    assert!(
        !content.starts_with("# user comment"),
        "header stripped: {content}"
    );
    assert!(content.contains("real body"), "body kept: {content}");
}

#[test]
fn test_rewrite_missing_editor_clean_error() {
    let env = TestEnv::new("fm-p3a-no-editor");
    let out = env.run_ok(&["add", "body", "--type", "idea"]);
    let id = extract_id(&out);

    let output = env.run_with_env(
        &["rewrite", &id],
        &[
            ("SIMARIS_EDITOR", "nonexistent-editor-binary-xyz"),
            ("EDITOR", ""),
        ],
    );
    assert!(
        !output.status.success(),
        "missing editor fails: {:?}",
        output
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked at"), "no panic: {stderr}");
}

#[test]
fn test_rewrite_wrong_type_field_rejected() {
    let env = TestEnv::new("fm-p3a-type-mismatch");
    let out = env.run_ok(&["add", "starting aspect body", "--type", "aspect"]);
    let id = extract_id(&out);

    // `trigger:` belongs to procedure, not aspect.
    let fixture = env.dir.join("mismatch.md");
    std::fs::write(&fixture, "---\ntrigger: weekly\n---\nbody\n").unwrap();

    let output = env.run_with_env(
        &["rewrite", &id],
        &[("SIMARIS_EDITOR", &editor_replaces_with(&fixture))],
    );
    assert!(
        !output.status.success(),
        "type mismatch rejects: {:?}",
        output
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not valid for unit type") || stderr.contains("aspect"),
        "err cites mismatch: {stderr}"
    );
}

// --- P3a.1 regression: cancel-without-edit must not mutate ------------

/// Dogfood bug (task `onqm`): on a prose unit, `rewrite` composes
/// skeleton+body into the buffer. Old no-op check compared buffer-final to
/// DB content (body-only) — so closing editor without edits still saw a
/// "diff" and wrote back the skeleton-prepended body, destroying the unit.
/// Fix: compare buffer-final to buffer-initial. Regression guard.
#[test]
fn test_rewrite_prose_noop_leaves_unit() {
    let env = TestEnv::new("fm-p3a1-prose-noop");
    let out = env.run_ok(&[
        "add",
        "original prose body with no frontmatter here",
        "--type",
        "aspect",
    ]);
    let id = extract_id(&out);
    let before = env.run_ok(&["show", &id, "--raw"]);

    let output = env.run_with_env(&["rewrite", &id], &[("SIMARIS_EDITOR", &editor_noop())]);
    assert!(output.status.success(), "prose noop exits 0: {:?}", output);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no-op") || stderr.contains("no changes"),
        "stderr announces no-op: {stderr}"
    );

    let after = env.run_ok(&["show", &id, "--raw"]);
    assert_eq!(before, after, "prose unit unchanged on no-edit cancel");
}

/// Even when the editor *writes* the buffer back unchanged (vim :wq with no
/// edits — touches mtime, same bytes), rewrite must treat it as a no-op.
/// Covers the "editor touched file but content identical" path for a prose
/// unit, where the initial buffer carries a skeleton that's not in the DB.
#[test]
fn test_rewrite_prose_identical_rewrite_is_noop() {
    let env = TestEnv::new("fm-p3a1-prose-identical");
    let out = env.run_ok(&["add", "prose body kept verbatim", "--type", "procedure"]);
    let id = extract_id(&out);
    let before = env.run_ok(&["show", &id, "--raw"]);

    // Editor reads the seed and writes it back byte-for-byte.
    let editor_cmd = "sh -c 'cat \"$1\" > \"$1.tmp\" && mv \"$1.tmp\" \"$1\"' --";
    let output = env.run_with_env(&["rewrite", &id], &[("SIMARIS_EDITOR", editor_cmd)]);
    assert!(
        output.status.success(),
        "identical rewrite exits 0: {:?}",
        output
    );

    let after = env.run_ok(&["show", &id, "--raw"]);
    assert_eq!(before, after, "prose unit unchanged on identical rewrite");
}

// --- P3b rewrite --suggest (LLM pre-fill) ------------------------------

/// Build a stub `claude` shell script and return the directory containing it.
/// Caller prepends this dir to `PATH`. The stub: optionally writes a fixed
/// stdout payload, optionally exits non-zero, optionally records its argv.
fn claude_stub_dir(env: &TestEnv, name: &str, body: &str) -> std::path::PathBuf {
    let dir = env.dir.join(format!("stub-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("claude");
    std::fs::write(&path, body).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    use std::os::unix::fs::PermissionsExt;
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    dir
}

/// Helper: prepend stub dir to PATH so simaris finds the fake `claude` first.
fn path_with_stub(stub_dir: &std::path::Path) -> String {
    let existing = std::env::var("PATH").unwrap_or_default();
    format!("{}:{existing}", stub_dir.display())
}

#[test]
fn test_rewrite_suggest_dry_run_happy_stdout() {
    let env = TestEnv::new("fm-p3b-dry-run-happy");
    let out = env.run_ok(&["add", "raw aspect prose body", "--type", "aspect"]);
    let id = extract_id(&out);

    // Stub claude → emits a valid LLM-style rewrite.
    let payload = "---\nrole: \"test role\"\ndispatches_to: []\nhandles_directly: []\nrefs: []\n---\n\n# Test aspect\n\nrewritten body line\n";
    let stub_body =
        format!("#!/bin/sh\ncat <<'__SIMARIS_FIXTURE_END__'\n{payload}__SIMARIS_FIXTURE_END__\n");
    let stub_dir = claude_stub_dir(&env, "happy", &stub_body);

    let output = env.run_with_env(
        &["rewrite", "--suggest", &id, "--dry-run"],
        &[("PATH", &path_with_stub(&stub_dir))],
    );
    assert!(output.status.success(), "dry-run happy: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("role:"), "fm in stdout: {stdout}");
    assert!(
        stdout.contains("rewritten body"),
        "body in stdout: {stdout}"
    );

    // DB unchanged (still prose).
    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(
        raw.contains("raw aspect prose body"),
        "DB body untouched: {raw}"
    );
    assert!(!raw.contains("rewritten body"), "DB not written: {raw}");
}

#[test]
fn test_rewrite_suggest_dry_run_claude_missing_falls_back() {
    let env = TestEnv::new("fm-p3b-dry-run-no-claude");
    let out = env.run_ok(&["add", "fact body for fallback", "--type", "fact"]);
    let id = extract_id(&out);

    // Empty PATH: `which claude` fails.
    let output = env.run_with_env(
        &["rewrite", "--suggest", &id, "--dry-run"],
        &[("PATH", "/nonexistent")],
    );
    assert!(output.status.success(), "fallback exits 0: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("LLM failed") && stderr.contains("falling back"),
        "stderr cites fallback: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Skeleton + original body.
    assert!(stdout.contains("scope:"), "skeleton in stdout: {stdout}");
    assert!(
        stdout.contains("fact body for fallback"),
        "original body in stdout: {stdout}"
    );
}

#[test]
fn test_rewrite_suggest_dry_run_invalid_yaml_falls_back() {
    let env = TestEnv::new("fm-p3b-dry-run-bad-yaml");
    let out = env.run_ok(&["add", "principle body to convert", "--type", "principle"]);
    let id = extract_id(&out);

    // Stub claude → emits invalid YAML.
    let stub_body = "#!/bin/sh\nprintf '%s\\n' '---' ': : bad yaml :: :' '---' '' 'body'\n";
    let stub_dir = claude_stub_dir(&env, "bad-yaml", stub_body);

    let output = env.run_with_env(
        &["rewrite", "--suggest", &id, "--dry-run"],
        &[("PATH", &path_with_stub(&stub_dir))],
    );
    assert!(output.status.success(), "fallback exits 0: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("LLM output invalid") && stderr.contains("falling back"),
        "stderr cites fallback: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tension:"), "skeleton in stdout: {stdout}");
    assert!(
        stdout.contains("principle body to convert"),
        "original body in stdout: {stdout}"
    );
}

#[test]
fn test_rewrite_suggest_dry_run_missing_fences_falls_back() {
    // LLM emits prose-only (no `---` fences) on a typed unit. Schema would
    // be silently lost — must refuse + fall back to skeleton.
    let env = TestEnv::new("fm-p3b-dry-run-no-fences");
    let out = env.run_ok(&["add", "fact body original", "--type", "fact"]);
    let id = extract_id(&out);

    let stub_body = "#!/bin/sh\nprintf '%s\\n' '# Heading only' 'body line'\n";
    let stub_dir = claude_stub_dir(&env, "no-fences", stub_body);

    let output = env.run_with_env(
        &["rewrite", "--suggest", &id, "--dry-run"],
        &[("PATH", &path_with_stub(&stub_dir))],
    );
    assert!(output.status.success(), "fallback exits 0: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("missing frontmatter fences"),
        "stderr cites fence miss: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("scope:"), "skeleton in stdout: {stdout}");
}

#[test]
fn test_rewrite_dry_run_without_suggest_errors() {
    let env = TestEnv::new("fm-p3b-dry-run-bad-flag");
    let out = env.run_ok(&["add", "some body", "--type", "fact"]);
    let id = extract_id(&out);

    let output = env.run(&["rewrite", "--dry-run", &id]);
    assert!(!output.status.success(), "clap requires fail: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--suggest") || stderr.contains("required") || stderr.contains("requires"),
        "stderr cites requires: {stderr}"
    );
}

#[test]
fn test_rewrite_suggest_editor_buffer_has_original_block() {
    // Capture the editor buffer to confirm the suggest flow seeds:
    // 3-line header + ORIGINAL block + LLM draft.
    let env = TestEnv::new("fm-p3b-editor-buffer");
    let out = env.run_ok(&[
        "add",
        "first line of original\nsecond line",
        "--type",
        "aspect",
    ]);
    let id = extract_id(&out);

    let payload = "---\nrole: \"captured\"\ndispatches_to: []\nhandles_directly: []\nrefs: []\n---\n\n# Captured\n\nLLM draft line\n";
    let stub_body =
        format!("#!/bin/sh\ncat <<'__SIMARIS_FIXTURE_END__'\n{payload}__SIMARIS_FIXTURE_END__\n");
    let stub_dir = claude_stub_dir(&env, "editor-cap", &stub_body);

    // Editor copies the seeded buffer to a capture file then exits without
    // changing the temp file → no-op rewrite.
    let capture = env.dir.join("captured.md");
    let editor_cmd = format!("sh -c 'cp \"$1\" \"{}\"' --", capture.display());

    env.run_with_env(
        &["rewrite", "--suggest", &id],
        &[
            ("PATH", &path_with_stub(&stub_dir)),
            ("SIMARIS_EDITOR", &editor_cmd),
        ],
    );

    let seen = std::fs::read_to_string(&capture).unwrap();
    assert!(
        seen.contains("# simaris rewrite -- id:"),
        "header present: {seen}"
    );
    assert!(seen.contains("# ORIGINAL BODY"), "ORIGINAL marker: {seen}");
    assert!(
        seen.contains("# first line of original"),
        "original l1 prefixed: {seen}"
    );
    assert!(
        seen.contains("# second line"),
        "original l2 prefixed: {seen}"
    );
    assert!(seen.contains("LLM draft line"), "LLM draft seeded: {seen}");
    assert!(
        seen.contains("role: \"captured\""),
        "LLM frontmatter seeded: {seen}"
    );
}

#[test]
fn test_rewrite_suggest_editor_save_writes_unit() {
    // Full editor flow: stub claude → suggest seeds buffer → editor saves →
    // P3a write path runs. Confirm DB has the LLM-drafted frontmatter.
    let env = TestEnv::new("fm-p3b-editor-save");
    let out = env.run_ok(&["add", "prose to upgrade", "--type", "aspect"]);
    let id = extract_id(&out);

    let payload = "---\nrole: \"upgraded\"\ndispatches_to: []\nhandles_directly: []\nrefs: []\n---\n\n# Upgraded\n\nbody saved\n";
    let stub_body =
        format!("#!/bin/sh\ncat <<'__SIMARIS_FIXTURE_END__'\n{payload}__SIMARIS_FIXTURE_END__\n");
    let stub_dir = claude_stub_dir(&env, "editor-save", &stub_body);

    // No-op editor (file already seeded with the LLM draft + header).
    let editor_cmd = "true";

    env.run_with_env(
        &["rewrite", "--suggest", &id],
        &[
            ("PATH", &path_with_stub(&stub_dir)),
            ("SIMARIS_EDITOR", editor_cmd),
        ],
    );

    let raw = env.run_ok(&["show", &id, "--raw"]);
    assert!(raw.contains("role: \"upgraded\""), "fm written: {raw}");
    assert!(raw.contains("body saved"), "body written: {raw}");
}

// ----------------------------------------------------------------------------
// `simaris prime` — LOD-1 directory + `--primary` aspect injection (v0.3.7)
// ----------------------------------------------------------------------------

#[test]
fn test_prime_empty_store_returns_no_sections() {
    // Fresh DB: search-and-expand returns nothing, prime short-circuits with
    // an empty section list. JSON shape must still be well-formed.
    let env = TestEnv::new("prime-empty");
    let out = env.run_ok(&["--json", "prime", "anything goes here"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    assert_eq!(parsed["task"], "anything goes here");
    assert!(
        parsed["sections"]
            .as_array()
            .expect("sections array")
            .is_empty(),
        "empty store yields no sections, got: {out}"
    );
    assert_eq!(parsed["unit_count"], 0, "got: {out}");
}

#[test]
fn test_prime_lod1_default_returns_directory_entries() {
    // Default LOD-1 contract: procedure / principle / fact-lesson-idea
    // surface as directory entries (full=false). Body bodies stay out.
    let env = TestEnv::new("prime-lod1-default");
    env.run_ok(&[
        "add",
        "octopus tentacle procedure body",
        "--type",
        "procedure",
    ]);
    env.run_ok(&["add", "octopus principle text body", "--type", "principle"]);
    env.run_ok(&["add", "octopus context fact body", "--type", "fact"]);

    let out = env.run_ok(&["--json", "prime", "octopus"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    let sections = parsed["sections"].as_array().expect("sections array");

    let labels: Vec<String> = sections
        .iter()
        .map(|s| s["label"].as_str().unwrap().to_string())
        .collect();
    assert!(labels.contains(&"Procedures".to_string()), "got: {out}");
    assert!(labels.contains(&"Principles".to_string()), "got: {out}");
    assert!(labels.contains(&"Context".to_string()), "got: {out}");

    for s in sections {
        for u in s["units"].as_array().unwrap() {
            assert_eq!(
                u["full"], false,
                "default LOD-1 unit must be a directory entry: section={} unit={}",
                s["label"], u
            );
        }
    }
}

#[test]
fn test_prime_preferences_always_full_body() {
    // Preferences are small + structural — prime always emits full bodies
    // for them, regardless of `--primary`.
    let env = TestEnv::new("prime-preferences-full");
    env.run_ok(&[
        "add",
        "octopus preference body content",
        "--type",
        "preference",
    ]);

    let out = env.run_ok(&["--json", "prime", "octopus"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    let pref_section = parsed["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .find(|s| s["label"] == "Preferences")
        .unwrap_or_else(|| panic!("Preferences section missing: {out}"));

    let units = pref_section["units"].as_array().unwrap();
    assert!(!units.is_empty(), "preference unit present: {out}");
    for u in units {
        assert_eq!(u["full"], true, "preference must always be full=true: {u}");
    }
}

#[test]
fn test_prime_aspect_default_is_directory_entry() {
    // Without `--primary`, an aspect surfaced via search lands as a directory
    // entry (full=false) — callers haven't asked for the body inline.
    let env = TestEnv::new("prime-aspect-default");
    env.run_ok(&["add", "octopus aspect body content", "--type", "aspect"]);

    let out = env.run_ok(&["--json", "prime", "octopus"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    let aspects = parsed["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .find(|s| s["label"] == "Aspects")
        .unwrap_or_else(|| panic!("Aspects section missing: {out}"));

    for u in aspects["units"].as_array().unwrap() {
        assert_eq!(
            u["full"], false,
            "default aspect must be directory entry: {u}"
        );
    }
}

#[test]
fn test_prime_primary_by_id_expands_aspect_to_full() {
    // `--primary <uuid>` resolves to the unit id and flips `full` to true
    // for that aspect only.
    let env = TestEnv::new("prime-primary-by-id");
    let add = env.run_ok(&["add", "octopus aspect body to expand", "--type", "aspect"]);
    let id = extract_id(&add);

    let out = env.run_ok(&["--json", "prime", "octopus", "--primary", &id]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    let aspects = parsed["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .find(|s| s["label"] == "Aspects")
        .unwrap_or_else(|| panic!("Aspects section missing: {out}"));

    let unit = aspects["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id)
        .unwrap_or_else(|| panic!("primary aspect missing from output: {out}"));
    assert_eq!(
        unit["full"], true,
        "primary aspect (by id) must be full=true: {unit}"
    );
}

#[test]
fn test_prime_primary_by_slug_expands_aspect_to_full() {
    // `--primary` also accepts slugs — main.rs forwards the raw string,
    // ask::prime resolves via db::resolve_id which checks the slugs table.
    let env = TestEnv::new("prime-primary-by-slug");
    let add = env.run_ok(&[
        "add",
        "octopus aspect referenced by slug",
        "--type",
        "aspect",
    ]);
    let id = extract_id(&add);
    env.run_ok(&["slug", "set", "my-aspect", &id]);

    let out = env.run_ok(&["--json", "prime", "octopus", "--primary", "my-aspect"]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    let aspects = parsed["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .find(|s| s["label"] == "Aspects")
        .unwrap_or_else(|| panic!("Aspects section missing: {out}"));

    let unit = aspects["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id)
        .unwrap_or_else(|| panic!("aspect resolved by slug missing from output: {out}"));
    assert_eq!(
        unit["full"], true,
        "primary aspect (by slug) must be full=true: {unit}"
    );
}

#[test]
fn test_prime_primary_aspect_injected_when_not_in_gather() {
    // Aspect with no FTS overlap with the task wouldn't reach Prime via
    // search/expand. The injection path in ask::prime fetches it directly
    // when the caller passes `--primary` — contract is "name it, get it".
    let env = TestEnv::new("prime-primary-inject");

    // Seed at least one matching unit so prime() doesn't short-circuit on
    // empty gather (the early-return predates the inject step by design).
    env.run_ok(&[
        "add",
        "octopus procedure body content",
        "--type",
        "procedure",
    ]);

    // Aspect body shares no terms with "octopus" → not in gather.
    let add = env.run_ok(&[
        "add",
        "zebra horse antelope unrelated body",
        "--type",
        "aspect",
    ]);
    let id = extract_id(&add);

    let out = env.run_ok(&["--json", "prime", "octopus", "--primary", &id]);
    let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    let aspects = parsed["sections"]
        .as_array()
        .expect("sections array")
        .iter()
        .find(|s| s["label"] == "Aspects")
        .unwrap_or_else(|| panic!("Aspects section missing — inject failed: {out}"));

    let unit = aspects["units"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["id"] == id)
        .unwrap_or_else(|| panic!("injected aspect missing from output: {out}"));
    assert_eq!(
        unit["full"], true,
        "injected primary aspect must be full=true: {unit}"
    );
    assert!(
        unit["content"].as_str().unwrap().contains("zebra"),
        "injected aspect carries its body: {unit}"
    );
}

#[test]
fn test_archive_hides_unit_from_default_views() {
    let env = TestEnv::new("archivehide");

    let live_out = env.run_ok(&["add", "live unit body uniqueA", "--type", "fact"]);
    let live_id = extract_id(&live_out);
    let dead_out = env.run_ok(&["add", "dead unit body uniqueB", "--type", "fact"]);
    let dead_id = extract_id(&dead_out);

    // Archive one of them.
    let arch_out = env.run_ok(&["archive", &dead_id]);
    assert!(
        arch_out.contains("Archived unit"),
        "archive should print confirmation: {arch_out}"
    );

    // Default list omits the archived row.
    let list_default = env.run_ok(&["list"]);
    assert!(
        list_default.contains("live unit body"),
        "default list shows live unit: {list_default}"
    );
    assert!(
        !list_default.contains("dead unit body"),
        "default list hides archived unit: {list_default}"
    );

    // --include-archived surfaces it with a marker.
    let list_inc = env.run_ok(&["list", "--include-archived"]);
    assert!(
        list_inc.contains("dead unit body"),
        "--include-archived list shows archived unit: {list_inc}"
    );
    assert!(
        list_inc.contains("[archived]"),
        "archived row should be visually marked: {list_inc}"
    );

    // Default search also hides; --include-archived restores.
    let search_default = env.run_ok(&["search", "uniqueB"]);
    assert!(
        !search_default.contains("dead unit body"),
        "default search hides archived unit: {search_default}"
    );
    let search_inc = env.run_ok(&["search", "uniqueB", "--include-archived"]);
    assert!(
        search_inc.contains("dead unit body"),
        "--include-archived search restores archived unit: {search_inc}"
    );

    // Show always works on archived units.
    let show_out = env.run_ok(&["show", &dead_id]);
    assert!(
        show_out.contains("dead unit body"),
        "show works on archived unit: {show_out}"
    );
    assert!(
        show_out.contains("[archived]"),
        "show surfaces archived state: {show_out}"
    );

    // Unarchive restores the unit to default views.
    let unarch_out = env.run_ok(&["unarchive", &dead_id]);
    assert!(
        unarch_out.contains("Unarchived unit"),
        "unarchive prints confirmation: {unarch_out}"
    );
    let list_after = env.run_ok(&["list"]);
    assert!(
        list_after.contains("dead unit body"),
        "unarchived unit reappears in default list: {list_after}"
    );

    // Sanity: live unit untouched throughout.
    let _ = live_id;
}

#[test]
fn test_archive_unarchive_idempotent_and_unknown() {
    let env = TestEnv::new("archiveidem");
    let out = env.run_ok(&["add", "subject", "--type", "fact"]);
    let id = extract_id(&out);

    // Double-archive succeeds (idempotent).
    env.run_ok(&["archive", &id]);
    env.run_ok(&["archive", &id]);

    // Double-unarchive succeeds (idempotent).
    env.run_ok(&["unarchive", &id]);
    env.run_ok(&["unarchive", &id]);

    // Unknown ID: archive/unarchive must error.
    let bad = env.run(&["archive", "no-such-unit"]);
    assert!(
        !bad.status.success(),
        "archive on unknown id must fail: {:?}",
        bad
    );
    let bad = env.run(&["unarchive", "no-such-unit"]);
    assert!(
        !bad.status.success(),
        "unarchive on unknown id must fail: {:?}",
        bad
    );
}

#[test]
fn test_stats_json_shape_and_counts() {
    let env = TestEnv::new("stats");

    // Seed: 3 facts (one shared tag), 1 procedure, 1 idea, 1 inbox drop,
    // 1 mark, one supersedes edge, one archived unit.
    let f1 = extract_id(&env.run_ok(&[
        "add",
        "fact one body",
        "--type",
        "fact",
        "--tags",
        "rust,cli",
    ]));
    let f2 = extract_id(&env.run_ok(&["add", "fact two body", "--type", "fact", "--tags", "rust"]));
    let f3 = extract_id(&env.run_ok(&["add", "fact three body", "--type", "fact"]));
    let _p1 = extract_id(&env.run_ok(&[
        "add",
        "procedure body that is long enough to satisfy the size warning baseline because procedures need verifiable steps and trigger context",
        "--type",
        "procedure",
        "--trigger",
        "on event",
        "--check",
        "verify outcome",
    ]));
    let _i1 = extract_id(&env.run_ok(&["add", "idea body", "--type", "idea", "--tags", "rust"]));
    env.run_ok(&["drop", "raw inbox content"]);
    env.run_ok(&["mark", &f1, "--kind", "used"]);
    env.run_ok(&["link", &f2, &f1, "--rel", "supersedes"]);
    let archived =
        extract_id(&env.run_ok(&["add", "to be archived", "--type", "fact", "--tags", "tmp"]));
    env.run_ok(&["archive", &archived]);

    // Default: live-only view.
    let raw = env.run_ok(&["stats", "--json"]);
    let stats: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON: {raw}");

    assert_eq!(stats["include_archived"], false);
    // 3 facts + 1 procedure + 1 idea = 5 live; archived not counted.
    assert_eq!(stats["total"], 5, "live total mismatch: {stats}");
    assert_eq!(
        stats["archived_count"], 1,
        "archived count mismatch: {stats}"
    );
    assert_eq!(stats["inbox_size"], 1);

    // by_type: archived (a fact) excluded → fact == 3.
    assert_eq!(stats["by_type"]["fact"], 3);
    assert_eq!(stats["by_type"]["procedure"], 1);
    assert_eq!(stats["by_type"]["idea"], 1);
    assert!(
        stats["by_type"].get("aspect").is_none(),
        "no aspect units seeded"
    );

    // marks: one `used` mark, no others.
    assert_eq!(stats["marks"]["used"], 1);
    assert!(stats["marks"].get("wrong").is_none() || stats["marks"]["wrong"] == 0);

    // superseded_count: f1 has one incoming `supersedes` link (from f2).
    assert_eq!(stats["superseded_count"], 1);

    // by_tag: `rust` appears on 2 live units (f1 + f2 + idea), but archived
    // tmp tag is excluded. The procedure has no tags.
    let top = stats["by_tag"]["top"].as_array().expect("top is array");
    let rust_count = top
        .iter()
        .find(|t| t["tag"] == "rust")
        .map(|t| t["count"].as_u64().unwrap_or(0))
        .unwrap_or(0);
    assert_eq!(rust_count, 3, "rust tag should be on f1, f2, idea: {stats}");
    let tmp_in_top = top.iter().any(|t| t["tag"] == "tmp");
    assert!(!tmp_in_top, "archived `tmp` tag must be excluded: {stats}");

    // confidence histogram: all newly-added units default to confidence 1.0.
    let conf = &stats["confidence"];
    assert_eq!(
        conf["verified"].as_u64().unwrap(),
        5,
        "all 5 live units default to ≥0.95: {stats}"
    );
    assert_eq!(conf["low"].as_u64().unwrap(), 0);
    assert_eq!(conf["medium"].as_u64().unwrap(), 0);
    assert_eq!(conf["high"].as_u64().unwrap(), 0);

    // by_type.fact must agree with `simaris list --type fact --json | jq length`.
    let list_facts = env.run_ok(&["list", "--type", "fact", "--json"]);
    let list_facts: Vec<serde_json::Value> = serde_json::from_str(&list_facts).expect("valid JSON");
    assert_eq!(stats["by_type"]["fact"], list_facts.len() as u64);

    // --include-archived rolls the archived fact back in.
    let raw_inc = env.run_ok(&["stats", "--json", "--include-archived"]);
    let stats_inc: serde_json::Value = serde_json::from_str(&raw_inc).expect("valid JSON");
    assert_eq!(stats_inc["include_archived"], true);
    assert_eq!(stats_inc["total"], 6);
    assert_eq!(stats_inc["by_type"]["fact"], 4);
    let top_inc = stats_inc["by_tag"]["top"].as_array().unwrap();
    let tmp_in_top_inc = top_inc.iter().any(|t| t["tag"] == "tmp");
    assert!(
        tmp_in_top_inc,
        "archived `tmp` tag must surface with --include-archived"
    );

    // archived_count is identical regardless of --include-archived.
    assert_eq!(stats_inc["archived_count"], 1);

    // sanity-touch other ids so unused-binding warnings don't fire.
    let _ = (f3,);
}

#[test]
fn test_stats_top_flag_caps_tag_list() {
    let env = TestEnv::new("statstop");
    // Seed 5 distinct tags across 5 units.
    for tag in ["a", "b", "c", "d", "e"] {
        env.run_ok(&[
            "add",
            &format!("body {tag}"),
            "--type",
            "fact",
            "--tags",
            tag,
        ]);
    }
    let raw = env.run_ok(&["stats", "--json", "--top", "2"]);
    let stats: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
    let top = stats["by_tag"]["top"].as_array().expect("array");
    assert_eq!(top.len(), 2, "--top 2 caps the list: {stats}");
    // total_unique still reflects the full count.
    assert_eq!(stats["by_tag"]["total_unique"], 5);
}

#[test]
fn test_stats_text_output_renders() {
    let env = TestEnv::new("statstext");
    env.run_ok(&["add", "lone fact", "--type", "fact"]);
    let out = env.run_ok(&["stats"]);
    assert!(out.contains("simaris stats"), "header present: {out}");
    assert!(out.contains("total:"), "total row present: {out}");
    assert!(out.contains("by type:"), "by type section: {out}");
    assert!(out.contains("confidence:"), "confidence section: {out}");
}

#[test]
fn test_stats_empty_db() {
    // Fresh DB with zero units must produce a valid (zero-filled) shape.
    let env = TestEnv::new("statsempty");
    let raw = env.run_ok(&["stats", "--json"]);
    let stats: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON: {raw}");
    assert_eq!(stats["total"], 0);
    assert_eq!(stats["archived_count"], 0);
    assert_eq!(stats["inbox_size"], 0);
    assert_eq!(stats["superseded_count"], 0);
    assert_eq!(stats["by_tag"]["total_unique"], 0);
    assert_eq!(stats["by_tag"]["top"].as_array().unwrap().len(), 0);
    // Histogram zero-filled, not null.
    assert_eq!(stats["confidence"]["low"], 0);
    assert_eq!(stats["confidence"]["medium"], 0);
    assert_eq!(stats["confidence"]["high"], 0);
    assert_eq!(stats["confidence"]["verified"], 0);
}

// `clone` returns a tag list of strings from a `simaris show --json` payload.
fn show_tags(env: &TestEnv, id: &str) -> Vec<String> {
    let raw = env.run_ok(&["show", id, "--json"]);
    let v: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");
    v["unit"]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap().to_string())
        .collect()
}

// Robust id extractor for `add` / `clone` text output. Handles the auto-link
// trailing line ("  auto-linked to N existing unit(s)") which breaks the
// generic `extract_id` helper that assumes a single-line response.
fn extract_first_line_id(output: &str) -> String {
    let first = output.lines().next().expect("non-empty output");
    first.split_whitespace().last().unwrap().to_string()
}

#[test]
fn test_clone_basic_copies_fields_and_resets_confidence() {
    let env = TestEnv::new("clonebasic");

    // Source with marks lowering confidence — clone must reset to default.
    let src = extract_id(&env.run_ok(&[
        "add",
        "source body",
        "--type",
        "fact",
        "--source",
        "manual",
        "--tags",
        "rust,cli",
    ]));
    env.run_ok(&["mark", &src, "--kind", "wrong"]); // -0.2 → 0.8

    let out = env.run_ok(&["clone", &src]);
    assert!(out.starts_with("Cloned "), "text output shape: {out}");
    let new_id = extract_first_line_id(&out);
    assert_uuid_format(&new_id);
    assert_ne!(new_id, src, "clone must mint fresh UUID");

    // New unit: same content/type/source/tags.
    let new_raw = env.run_ok(&["show", &new_id, "--json"]);
    let new_v: serde_json::Value = serde_json::from_str(&new_raw).expect("valid JSON");
    assert_eq!(new_v["unit"]["content"], "source body");
    assert_eq!(new_v["unit"]["type"], "fact");
    assert_eq!(new_v["unit"]["source"], "manual");
    let new_tags = show_tags(&env, &new_id);
    assert!(new_tags.contains(&"rust".to_string()));
    assert!(new_tags.contains(&"cli".to_string()));

    // Confidence reset to system default; verified false.
    assert_eq!(new_v["unit"]["confidence"], 1.0);
    assert_eq!(new_v["unit"]["verified"], false);

    // Source unit is untouched (still 0.8 confidence after the `wrong` mark).
    let src_raw = env.run_ok(&["show", &src, "--json"]);
    let src_v: serde_json::Value = serde_json::from_str(&src_raw).expect("valid JSON");
    assert_eq!(src_v["unit"]["content"], "source body");
    assert_eq!(src_v["unit"]["type"], "fact");
    let src_conf = src_v["unit"]["confidence"].as_f64().unwrap();
    assert!(
        (src_conf - 0.8).abs() < 1e-9,
        "source confidence unchanged: got {src_conf}"
    );
}

#[test]
fn test_clone_with_overrides() {
    let env = TestEnv::new("cloneoverride");
    let src = extract_id(&env.run_ok(&[
        "add",
        "body x",
        "--type",
        "fact",
        "--source",
        "orig",
        "--tags",
        "alpha,beta",
    ]));

    let out = env.run_ok(&[
        "clone", &src, "--type", "idea", "--source", "fork", "--tags", "gamma",
    ]);
    let new_id = extract_first_line_id(&out);

    let new_raw = env.run_ok(&["show", &new_id, "--json"]);
    let new_v: serde_json::Value = serde_json::from_str(&new_raw).expect("valid JSON");
    assert_eq!(new_v["unit"]["type"], "idea");
    assert_eq!(new_v["unit"]["source"], "fork");
    assert_eq!(new_v["unit"]["content"], "body x", "body unchanged");
    assert_eq!(show_tags(&env, &new_id), vec!["gamma".to_string()]);
}

#[test]
fn test_clone_does_not_copy_links_or_marks_and_auto_links() {
    let env = TestEnv::new("clonenolinks");

    // Three units sharing tags so auto-link will fire on the clone:
    //   a (rust,cli), b (rust,cli), c (rust,cli)
    // `add` prints an extra "auto-linked to N" line when the new unit shares
    // 2+ tags with existing ones — use the first-line extractor.
    let a = extract_first_line_id(&env.run_ok(&[
        "add",
        "alpha body",
        "--type",
        "fact",
        "--tags",
        "rust,cli",
    ]));
    let b = extract_first_line_id(&env.run_ok(&[
        "add",
        "beta body",
        "--type",
        "fact",
        "--tags",
        "rust,cli",
    ]));
    let _c = extract_first_line_id(&env.run_ok(&[
        "add",
        "gamma body",
        "--type",
        "fact",
        "--tags",
        "rust,cli",
    ]));

    // Give `a` an explicit outgoing link + a mark so we can verify those
    // do NOT carry over to the clone.
    env.run_ok(&["link", &a, &b, "--rel", "supersedes"]);
    env.run_ok(&["mark", &a, "--kind", "used"]);

    let out = env.run_ok(&["clone", &a]);
    let new_id = extract_first_line_id(&out);

    // Mark must not have been copied — confidence at default (1.0), not
    // bumped by `used`.
    let new_raw = env.run_ok(&["show", &new_id, "--json"]);
    let new_v: serde_json::Value = serde_json::from_str(&new_raw).expect("valid JSON");
    assert_eq!(new_v["unit"]["confidence"], 1.0);

    // The explicit `supersedes` link from a→b must not appear on the clone.
    let outgoing = new_v["links"]["outgoing"].as_array().unwrap();
    let has_supersedes = outgoing
        .iter()
        .any(|l| l["relationship"] == "supersedes" && l["to_id"] == b);
    assert!(
        !has_supersedes,
        "explicit links must not be copied to clone: {outgoing:?}"
    );

    // But auto-link fires — clone shares 2+ tags with a, b, c → at least one
    // related_to edge from the clone.
    let related_count = outgoing
        .iter()
        .filter(|l| l["relationship"] == "related_to")
        .count();
    assert!(
        related_count >= 2,
        "auto-link must fire (≥2 related_to edges expected): {outgoing:?}"
    );

    // Source unit `a` still has its supersedes edge to b.
    let src_raw = env.run_ok(&["show", &a, "--json"]);
    let src_v: serde_json::Value = serde_json::from_str(&src_raw).expect("valid JSON");
    let src_outgoing = src_v["links"]["outgoing"].as_array().unwrap();
    let src_has_supersedes = src_outgoing
        .iter()
        .any(|l| l["relationship"] == "supersedes" && l["to_id"] == b);
    assert!(
        src_has_supersedes,
        "source unit links must remain intact: {src_outgoing:?}"
    );
}

#[test]
fn test_clone_json_output_shape() {
    let env = TestEnv::new("clonejson");
    let src = extract_id(&env.run_ok(&["add", "body", "--type", "fact"]));

    let raw = env.run_ok(&["--json", "clone", &src]);
    let v: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON: {raw}");
    let new_id = v["id"].as_str().expect("id field").to_string();
    let from_id = v["from"].as_str().expect("from field");
    assert_uuid_format(&new_id);
    assert_eq!(from_id, src);
    assert_ne!(new_id, src);
}

#[test]
fn test_clone_unknown_id_errors() {
    let env = TestEnv::new("clonebadid");
    let out = env.run(&["clone", "no-such-id"]);
    assert!(!out.status.success(), "must fail on unknown id");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("No unit or slug matches"),
        "stderr should mention resolution failure: {stderr}"
    );
}

#[test]
fn test_clone_resolves_slug() {
    let env = TestEnv::new("cloneslug");
    let src = extract_id(&env.run_ok(&["add", "slugged body", "--type", "fact"]));
    env.run_ok(&["slug", "set", "myslug", &src]);

    let out = env.run_ok(&["clone", "myslug"]);
    let new_id = extract_first_line_id(&out);
    assert_uuid_format(&new_id);
    assert_ne!(new_id, src);

    let new_raw = env.run_ok(&["show", &new_id, "--json"]);
    let new_v: serde_json::Value = serde_json::from_str(&new_raw).expect("valid JSON");
    assert_eq!(new_v["unit"]["content"], "slugged body");
}
