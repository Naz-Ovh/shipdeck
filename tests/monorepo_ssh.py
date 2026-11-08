#!/usr/bin/env python3
"""Exercise monorepo deployment through the CLI and RPC over SSH in Podman."""

import argparse
import json
import pathlib
import queue
import subprocess
import tempfile
import threading
import time


class Harness:
    """Own isolated test sources and a dedicated deployment inside a test container."""

    def __init__(self, args, root):
        self.args = args
        self.root = pathlib.Path(root)
        self.project = self.root / "product"
        self.project.mkdir()
        self.name = f"shipdeck-e2e-{int(time.time())}"
        self.remote = f"/tmp/{self.name}"
        self.log = self.root / "commands.log"
        for app in ("api", "web", "worker"):
            source = self.project / app
            source.mkdir()
            (source / "server.js").write_text(
                "console.log('started ' + process.env.PORT);\n"
                "require('http').createServer((req,res)=>res.end('v1')).listen(process.env.PORT);\n"
            )
            (source / ".env").write_text("EXAMPLE=loaded\n")
        (self.project / "VERSION").write_text("1.0.0\n")
        # Rootless Podman cannot apply the built-in unit's mount-namespace hardening.
        # Exercise normal systemd lifecycle with a test-only custom unit.
        (self.project / "service.template").write_text(
            "[Unit]\nDescription={{name}}\n[Service]\nType=simple\nUser={{user}}\n"
            "WorkingDirectory={{workdir}}\nExecStart={{exec}}\n{{env}}\n"
            "Restart=always\nRestartSec=2\n[Install]\nWantedBy=multi-user.target\n"
        )
        self.api_port = 33000 + (int(time.time()) % 1000) * 3
        self.write_config()

    def command(self, command, expected=0):
        result = subprocess.run(command, text=True, capture_output=True, timeout=120)
        with self.log.open("a") as log:
            log.write(f"\n$ {command!r}\n{result.stdout}{result.stderr}")
        assert result.returncode == expected, (
            f"exit {result.returncode}, expected {expected}: {command!r}\n"
            f"{result.stdout}{result.stderr}"
        )
        return result.stdout

    def remote_command(self, script):
        return self.command(["podman", "exec", self.args.container, "sh", "-c", script]).strip()

    def cli(self, action, *extra, expected=0):
        return self.command([
            self.args.binary, action, str(self.root), "product", *extra,
            "--key", self.args.key, "--trust-new-host",
        ], expected)

    def current(self):
        return self.remote_command(f"basename $(readlink {self.remote}/current)")

    def write_config(self, second=False, setup_fail=False, health_fail=False):
        """Use an external API config and inline apps with mixed managers."""
        api_manager = "pm2" if second else "systemd"
        api = f"""project: api
start: node server.js
process_manager: {{name: {api_manager}}}
shared: [uploads]
env_file: .env
env: {{PORT: '{self.api_port}'}}
pipeline:
  setup:
    run:
      server: 'test ! -e uploads/fail-restore && test "$EXAMPLE" = loaded && echo api >> ../../hooks.log && echo setup > setup.txt'
  backup: {{files: {{include: [setup.txt]}}}}
  health: {{url: 'http://127.0.0.1:{self.api_port}'}}
"""
        (self.project / "api.yaml").write_text(api)
        other = "worker" if second else "web"
        port = str(self.api_port + (2 if second else 1))
        start_line = "" if second else "    start: node server.js\n"
        manager = "{name: pm2}"
        app_options = ""
        if second:
            manager = "{name: pm2, ecosystem: ecosystem.json}"
            (self.project / "worker" / "ecosystem.json").write_text(json.dumps({"apps": [{
                "name": f"{self.name}-worker", "script": "server.js",
                "cwd": f"{self.remote}/current/apps/worker",
            }]}))
            app_options = "      pack: {files: {include: {project: [server.js, .env, ecosystem.json]}}}\n      backup: {files: {exclude: [ecosystem.json]}}\n"
        self.config = f"""name: {self.name}
ssh: {{host: 127.0.0.1, port: {self.args.port}, user: deploy, shell_env: false}}
dir: {self.remote}
pipeline:
  preflight: {{run: {{local: 'echo preflight >> order.log'}}}}
  build: {{version_from: VERSION, run: {{local: 'echo global >> order.log'}}}}
  pack: {{files: {{include: {{project: [server.js, .env]}}}}}}
  setup: {{run: {{server: '{'false' if setup_fail else 'echo global > hooks.log'}'}}}}
  backup: {{keep: {{open: 1, compressed: 3}}}}
  health: {{timeout_secs: 15, check: {{server: '{'false' if health_fail else 'test -f hooks.log'}'}}}}
  failed: {{run: {{local: 'echo "$SHIPDECK_STEP:$SHIPDECK_ERROR" >> failed.log'}}}}
apps:
  api: api.yaml
  {other}:
    project: {other}
{start_line}    process_manager: {manager}
    shared: [uploads]
    env: {{PORT: '{port}'}}
    pipeline:
{app_options}      build: {{run: {{local: 'echo {other} >> ../order.log'}}}}
      health: {{url: 'http://127.0.0.1:{port}'}}
"""
        (self.project / "shipdeck.yaml").write_text(self.config)

    def run(self):
        self.cli("deploy")
        first = self.current()
        assert self.remote_command(f"cat {self.remote}/current/hooks.log") == "global\napi"
        assert (self.project / "order.log").read_text() == "preflight\nglobal\nweb\n"
        self.remote_command(f"echo persistent > {self.remote}/current/apps/api/uploads/data")
        self.cli("deploy", expected=6)
        assert self.current() == first
        self.write_config(second=True)
        (self.project / "VERSION").write_text("2.0.0\n")
        self.cli("deploy")
        second = self.current()
        assert second != first
        assert self.remote_command(f"test ! -d {self.remote}/releases/{first} && echo closed") == "closed"
        assert self.remote_command(f"cat {self.remote}/current/apps/api/uploads/data") == "persistent"
        # A restore setup failure must preserve the backup and the current release.
        self.remote_command(f"touch {self.remote}/shared/apps/api/uploads/fail-restore")
        self.cli("rollback", "--to", first, expected=1)
        assert self.current() == second
        assert self.remote_command(f"test ! -d {self.remote}/releases/{first} && test -f {self.remote}/backups/{first}.tar.zst && echo preserved") == "preserved"
        self.remote_command(f"unlink {self.remote}/shared/apps/api/uploads/fail-restore")
        # Local config still describes v2; rollback must recover v1's systemd/web settings.
        self.cli("rollback", "--to", first)
        assert self.current() == first
        assert self.remote_command(f"cat {self.remote}/current/apps/api/setup.txt") == "setup"
        assert self.remote_command(f"systemctl is-active {self.name}-api") == "active"
        self.remote_command(f"runuser -u deploy -- sh -c 'pm2 jlist' > {self.remote}/pm2.json")
        processes = json.loads(self.remote_command(f"cat {self.remote}/pm2.json"))
        names = {app["name"] for app in processes}
        assert f"{self.name}-worker" not in names
        assert f"{self.name}-api" not in names
        assert f"{self.name}-web" in names
        self.write_config(setup_fail=True)
        (self.project / "VERSION").write_text("3.0.0\n")
        self.cli("deploy", expected=1)
        assert self.current() == first
        assert "setup:" in (self.project / "failed.log").read_text()
        self.write_config(second=True, health_fail=True)
        self.cli("deploy", expected=3)
        assert self.current() == first
        assert self.remote_command(f"systemctl is-active {self.name}-api") == "active"
        # A missing unit after setup fails halfway through activation.
        self.write_config()
        (self.project / "VERSION").write_text("4.0.0\n")
        (self.project / "shipdeck.yaml").write_text(self.config.replace(
            "echo global > hooks.log", f"echo global > hooks.log && rm {self.name}-api.service"))
        self.cli("deploy", expected=3)
        assert self.current() == first
        # Done-hook errors keep a healthy release and return a warning.
        self.write_config()
        (self.project / "VERSION").write_text("5.0.0\n")
        (self.project / "shipdeck.yaml").write_text(self.config.replace(
            "  failed:", "  done: {run: {server: 'false'}}\n  failed:"))
        self.cli("deploy")
        healthy = self.current()
        assert healthy != first
        self.cli("rollback", "--to", first)
        assert self.current() == first
        # Custom PM2 ecosystem settings must survive compression and backup exclusions.
        self.cli("rollback", "--to", second)
        assert self.current() == second
        self.cli("rollback", "--to", first)
        assert self.current() == first
        self.rpc_checks(first)
        print(f"PASS: deploy, unchanged, manager/app changes, shared data, retention, backup rollback, failed restore, setup failure, partial activation recovery, custom PM2 backups, done warnings, RPC controls/logs. Logs: {self.log}")

    def rpc_checks(self, first):
        with Rpc(self.args.binary) as rpc:
            rpc.call("initialize", {"protocol": 1, "client": "monorepo-ssh-test"})
            opened = rpc.call("workspace/open", {
                "root": str(self.root), "login": {"kind": "key", "path": self.args.key},
            })
            assert len(opened["deployables"]) == 1
            state = rpc.call("status/get", {"deployable": "product"})
            assert state["current"] == first and state["state"] == "active", state
            entries = rpc.call("releases/list", {"deployable": "product"})
            assert entries["current"] == first
            contents = rpc.call("release/contents", {"deployable": "product", "release": first, "source": "folder"})
            assert any(f["path"] == "shipdeck-monorepo.json" for f in contents["files"])
            text = rpc.call("release/read_file", {"deployable": "product", "release": first, "source": "folder", "path": "apps/api/setup.txt"})
            assert text["text"].strip() == "setup"
            # Release inspection remains available if live process metadata is damaged.
            metadata = f"{self.remote}/current/shipdeck-monorepo.json"
            self.remote_command(f"mv {metadata} {metadata}.test-saved")
            try:
                assert rpc.call("releases/list", {"deployable": "product"})["current"] == first
                inspected = rpc.call("release/read_file", {"deployable": "product", "release": first, "source": "folder", "path": "apps/api/setup.txt"})
                assert inspected["text"].strip() == "setup"
            finally:
                self.remote_command(f"mv {metadata}.test-saved {metadata}")
            rpc.call("service/control", {"deployable": "product", "action": "stop"})
            assert rpc.call("status/get", {"deployable": "product"})["state"] != "active"
            rpc.call("service/control", {"deployable": "product", "action": "start"})
            rpc.call("service/control", {"deployable": "product", "action": "restart"})
            view = rpc.call("logs/follow", {"deployable": "product"})["view"]
            labels = set()
            deadline = time.monotonic() + 20
            while len(labels) < 2 and time.monotonic() < deadline:
                message = rpc.read()
                if message.get("method") == "logs/line":
                    labels.add(message["params"]["line"].split("]")[0])
            assert len(labels) == 2, labels
            rpc.call("logs/stop", {"view": view})
            rpc.call("connection/close", {"deployable": "product"})


class Rpc:
    """Read JSON-RPC messages with bounded waits and automatic host trust."""

    def __init__(self, binary):
        self.process = subprocess.Popen([binary, "engine"], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, text=True)
        self.messages = queue.Queue()
        self.sequence = 0
        threading.Thread(target=self.pump, daemon=True).start()

    def pump(self):
        for line in self.process.stdout:
            self.messages.put(json.loads(line))

    def read(self):
        return self.messages.get(timeout=30)

    def write(self, message):
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", **message}) + "\n")
        self.process.stdin.flush()

    def call(self, method, params):
        self.sequence += 1
        identifier = self.sequence
        self.write({"id": identifier, "method": method, "params": params})
        while True:
            message = self.read()
            if "method" in message and "id" in message:
                assert message["method"] == "ask/trust_host", message
                self.write({"id": message["id"], "result": True})
            elif message.get("id") == identifier:
                assert "error" not in message, message
                return message["result"]

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.process.stdin.close()
        self.process.wait(timeout=20)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/shipdeck")
    parser.add_argument("--container", default="shipdeck-monorepo-ssh-test")
    parser.add_argument("--port", type=int, default=38733)
    parser.add_argument("--key", required=True)
    args = parser.parse_args()
    args.binary = str(pathlib.Path(args.binary).resolve())
    args.key = str(pathlib.Path(args.key).resolve())
    # Keep logs and sources to make failures reproducible.
    root = tempfile.mkdtemp(prefix="shipdeck-monorepo-e2e-")
    print(f"Test files: {root}", flush=True)
    Harness(args, root).run()


if __name__ == "__main__":
    main()
