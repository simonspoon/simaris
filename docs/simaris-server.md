# Running simaris-server as a launchd service

`simaris-server` is the HTTP admin dashboard for the simaris knowledge store. It binds `0.0.0.0:3535` and shells out to the `simaris` CLI for all data and mutations. This guide covers running it permanently in the background on macOS via `launchd`.

## 1. Install the binaries

Build and install both `simaris` and `simaris-server` to `~/.cargo/bin/`:

```sh
cd ~/claudehub/simaris
cargo install --path .                # installs simaris
cargo install --path ./simaris-server # installs simaris-server
```

Verify:

```sh
which simaris        # expect: /Users/<you>/.cargo/bin/simaris
which simaris-server # expect: /Users/<you>/.cargo/bin/simaris-server
```

The server resolves the `simaris` binary via the `SIMARIS_BIN` env var, falling back to `simaris` on `PATH`. Set `SIMARIS_BIN` explicitly in the plist below — `launchd` does not inherit your shell's `PATH`.

## 2. Create the LaunchAgent plist

Save the following as `~/Library/LaunchAgents/com.sjspoon.simaris-server.plist`. Replace `<you>` with your username (or use `echo $HOME`).

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.sjspoon.simaris-server</string>

    <key>ProgramArguments</key>
    <array>
        <string>/Users/<you>/.cargo/bin/simaris-server</string>
    </array>

    <key>EnvironmentVariables</key>
    <dict>
        <key>SIMARIS_HOME</key>
        <string>/Users/<you>/.simaris</string>
        <key>SIMARIS_BIN</key>
        <string>/Users/<you>/.cargo/bin/simaris</string>
        <key>RUST_LOG</key>
        <string>info</string>
    </dict>

    <key>StandardOutPath</key>
    <string>/Users/<you>/Library/Logs/simaris-server/stdout.log</string>

    <key>StandardErrorPath</key>
    <string>/Users/<you>/Library/Logs/simaris-server/stderr.log</string>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <true/>

    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
```

Key choices:

- **`RunAtLoad=true`** — start when the agent loads (login or `launchctl load`).
- **`KeepAlive=true`** — relaunch on crash. Desirable for an always-on admin tool.
- **`SIMARIS_HOME`** — data dir; defaults to `~/.simaris` if unset, but `launchd` has no `$HOME` resolution at parse time, so set the absolute path.
- **`SIMARIS_BIN`** — absolute path to the `simaris` CLI; `launchd` does not see your shell `PATH`.
- **`RUST_LOG=info`** — server uses `tracing-subscriber` with `EnvFilter`. Bump to `debug` for troubleshooting.

Make sure the log directory exists before loading:

```sh
mkdir -p ~/Library/Logs/simaris-server
```

## 3. Load and start the service

```sh
launchctl load ~/Library/LaunchAgents/com.sjspoon.simaris-server.plist
```

Confirm it's running:

```sh
launchctl list | grep simaris-server
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:3535/healthz   # expect 200
```

Check logs if it fails to start:

```sh
tail -f ~/Library/Logs/simaris-server/stderr.log
```

To stop:

```sh
launchctl unload ~/Library/LaunchAgents/com.sjspoon.simaris-server.plist
```

The service will restart automatically on reboot because the plist lives in `~/Library/LaunchAgents/` and `RunAtLoad=true`.

## 4. Network and firewall

The server binds `0.0.0.0:3535`, so it is reachable from any device on the same network. There is **no authentication** — anyone on the LAN who can reach port `3535` can browse, edit, clone, and archive units.

This is acceptable only on a **trusted home/office LAN**. If you are on a coffee-shop Wi-Fi, a co-working space, or a corporate network you do not control, do one of:

- Stop the service: `launchctl unload …`
- Block port 3535 in macOS firewall (System Settings → Network → Firewall → Options → block incoming for `simaris-server`).
- Bind to `127.0.0.1` only (requires a code change — not currently configurable).

To reach the dashboard from another device on the LAN:

```
http://<your-mac-hostname>.local:3535/
```

## 5. Upgrading

After rebuilding the binary with `cargo install --path ./simaris-server`, kick the running service so it picks up the new executable:

```sh
launchctl kickstart -k gui/$(id -u)/com.sjspoon.simaris-server
```

`-k` sends `SIGTERM` first, waits for graceful shutdown (the server handles `SIGTERM` via `tokio::signal`), then relaunches. Verify:

```sh
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:3535/healthz
```

If you change the plist itself (env vars, log paths, etc.), unload and load again — `kickstart` re-reads the binary but not the plist.

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| `launchctl load` silent, no process | plist syntax error | `plutil -lint ~/Library/LaunchAgents/com.sjspoon.simaris-server.plist` |
| `curl` to `:3535` connection refused | service crashed | `tail ~/Library/Logs/simaris-server/stderr.log` |
| Routes return 500 with "spawn simaris" errors | `SIMARIS_BIN` wrong or unset | check `EnvironmentVariables` block; absolute path required |
| Service flapping (restart loop) | crash on boot — usually `SIMARIS_HOME` permissions | verify the dir exists and is writable by your user |
