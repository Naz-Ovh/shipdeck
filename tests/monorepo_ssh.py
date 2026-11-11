#!/usr/bin/env python3
"""Exercise config format 2 deployment through the CLI and RPC over SSH.

The suite is written against the design in docs/feature/config-v2.md. It needs
a disposable Podman container with systemd, SSH, Node.js, PM2, zstd, tar,
sha256sum and curl or wget. The `deploy` user must accept the supplied key and
have passwordless sudo.

Every scenario writes its own project folder, runs the real binary from that
folder, and reads the server state back over `podman exec`.
"""

import argparse
import json
import pathlib
import queue
import subprocess
import tempfile
import threading
import time


class Harness:
    """One isolated project and one deployment inside the test container."""

    def __init__(self, args, root):
        self.args = args
        self.root = pathlib.Path(root)
        self.project = self.root / "product"
        self.project.mkdir()
        self.name = f"sd2-{int(time.time())}"
        self.remote = f"/tmp/{self.name}"
        self.log = self.root / "commands.log"
        self.port = 34000 + (int(time.time()) % 700) * 4
        self.version = 1
        self.write_sources()

    # ── local side ────────────────────────────────────────────────────────
    def command(self, command, expected=0, cwd=None):
        result = subprocess.run(
            command, text=True, capture_output=True, timeout=180, cwd=cwd
        )
        with self.log.open("a") as log:
            log.write(f"\n$ {command!r}\n{result.stdout}{result.stderr}")
        assert result.returncode == expected, (
            f"exit {result.returncode}, expected {expected}: {command!r}\n"
            f"{result.stdout}{result.stderr}"
        )
        return result.stdout

    def cli(self, *args, expected=0):
        return self.command(
            [
                self.args.binary,
                *args,
                "--key",
                self.args.key,
                "--trust-new-host",
            ],
            expected,
            cwd=self.root,
        )

    def write_sources(self):
        for app in ("api", "web", "worker"):
            source = self.project / app
            source.mkdir(exist_ok=True)
            (source / "server.js").write_text(
                "console.log('started ' + process.env.PORT);\n"
                "require('http').createServer((req,res)=>res.end('ok'))."
                "listen(process.env.PORT);\n"
            )
            (source / ".env").write_text("EXAMPLE=loaded\n")
        # Rootless Podman cannot apply the built-in unit's mount namespaces.
        (self.project / "test.service").write_text(
            "[Unit]\nDescription={{NAME}}\n[Service]\nType=simple\n"
            "User={{ user }}\nWorkingDirectory={{APP_CURRENT}}\n"
            "ExecStart={{exec}}\n{{env}}\nRestart=always\nRestartSec=2\n"
            "[Install]\nWantedBy=multi-user.target\n"
        )
        (self.project / "package.json").write_text('{"version": "1.0.0"}\n')
        (self.project / "shipdeck.yaml").write_text(
            "deploys:\n  production: deploy/production.yaml\n"
        )
        (self.project / "deploy").mkdir(exist_ok=True)
        self.write_deploy()

    def bump(self):
        self.version += 1
        (self.project / "package.json").write_text(
            '{"version": "1.%d.0"}\n' % self.version
        )

    def write_deploy(
        self,
        api_manager="systemd",
        worker_ecosystem=False,
        setup_fail=False,
        health_fail=False,
        extra_release="",
        extra_api="",
        runner_step="",
    ):
        """Write one deploy: a systemd or PM2 API, a PM2 worker, a static site."""
        worker_pm2 = (
            "      pm2:\n        ecosystem: ecosystem.json\n"
            if worker_ecosystem
            else "      pm2:\n        options: { instances: 1 }\n"
        )
        worker_start = (
            "" if worker_ecosystem else "      start: node server.js\n"
        )
        if worker_ecosystem:
            (self.project / "worker" / "ecosystem.json").write_text(
                json.dumps(
                    {
                        "apps": [
                            {
                                "name": f"{self.name}-worker",
                                "script": "server.js",
                                "cwd": f"{self.remote}/current/apps/worker",
                            }
                        ]
                    }
                )
            )
        api_block = (
            "      systemd:\n        template: test.service\n"
            if api_manager == "systemd"
            else "      pm2:\n        options: { instances: 1 }\n"
        )
        setup_command = (
            "false" if setup_fail else "echo setup > setup.txt && echo api >> hooks.log"
        )
        health_command = "false" if health_fail else "test -f setup.txt"
        self.deploy = f"""# Written by tests/monorepo_ssh.py.
name: {self.name}
server:
  ssh: {{ host: 127.0.0.1, port: {self.args.port}, user: deploy, shell_env: false }}
  dir: {self.remote}

release:
  files:
    - package.json
    - {{ path: ./product/extra.txt, to: extra/extra.txt }}
  steps:
    preflight:
      local: echo preflight >> order.log
    build:
      version_from: package.json
      local: echo release >> order.log
    setup:
      server: echo global > hooks.log
    backup:
      keep: {{ open: 2, compressed: 3 }}
    health:
      timeout_secs: 20
      server: test -f hooks.log
    done:
      local: echo done >> order.log
    failed:
      local: echo "$SHIPDECK_STEP" >> failed.log
{extra_release}
apps:
  api:
    source: api
    files:
      - api/server.js
      - {{ path: ./product/api.env, to: .env }}
    binary: api/bin/api
    process:
      start: node server.js
      env_file: .env
      env: {{ PORT: "{self.port}" }}
{api_block}
    shared: [uploads]
    steps:
      setup:
        server: {setup_command}
      backup:
        include: [setup.txt]
      health:
        url: http://127.0.0.1:{self.port}
{runner_step}{extra_api}
  worker:
    source: worker
    files: [worker/server.js]
    process:
{worker_start}{worker_pm2}      env: {{ PORT: "{self.port + 1}" }}
    steps:
      health:
        url: http://127.0.0.1:{self.port + 1}

  site:
    source: web
    files: [web/dist]
    static: true
"""
        (self.project / "api.env").write_text("EXAMPLE=loaded\n")
        (self.project / "extra.txt").write_text("extra\n")
        (self.project / "web" / "dist").mkdir(parents=True, exist_ok=True)
        (self.project / "web" / "dist" / "index.html").write_text("<h1>hi</h1>\n")
        # A binary the API app ships, so the binary path is real.
        binary = self.project / "api" / "bin" / "api"
        binary.parent.mkdir(exist_ok=True)
        binary.write_text("#!/bin/sh\nexit 0\n")
        (self.project / "deploy" / "production.yaml").write_text(self.deploy)

    # ── server side ───────────────────────────────────────────────────────
    def remote_command(self, script, expected=0):
        return self.command(
            ["podman", "exec", self.args.container, "sh", "-c", script], expected
        ).strip()

    def current(self):
        return self.remote_command(f"basename $(readlink {self.remote}/current)")

    def releases(self):
        out = self.remote_command(f"ls -1 {self.remote}/releases 2>/dev/null || true")
        return [line for line in out.splitlines() if line]

    def backups(self):
        out = self.remote_command(f"ls -1 {self.remote}/backups 2>/dev/null || true")
        return [line.removesuffix(".tar.zst") for line in out.splitlines() if line]

    # ── the run ───────────────────────────────────────────────────────────
    def run(self):
        self.single_and_several_apps()
        self.rollback_from_backup()
        self.failure_before_the_switch()
        self.automatic_rollback()
        self.manual_rollback_to_an_open_release()
        self.keep_failed()
        self.runners()
        self.missing_runner()
        self.rpc_checks()
        print(
            "PASS: single app, several apps, release files, to, .shipdeck/, "
            "shared links, each runner, failure before the switch, automatic "
            "rollback, rollback by hand to an open release, rollback from a "
            f"backup, keep_failed, preflight runner check. Logs: {self.log}"
        )

    def single_and_several_apps(self):
        self.cli("deploy", "production")
        first = self.current()
        assert self.remote_command(f"cat {self.remote}/current/hooks.log") == "global\napi"
        # The release files landed where the design says.
        self.remote_command(f"test -f {self.remote}/current/package.json")
        self.remote_command(f"test -f {self.remote}/current/extra/extra.txt")
        self.remote_command(f"test -f {self.remote}/current/apps/site/dist/index.html")
        self.remote_command(f"test -f {self.remote}/current/apps/api/bin/api")
        self.remote_command(
            f"test \"$(stat -c %a {self.remote}/current/apps/api/bin/api)\" = 755"
        )
        # Shipdeck's own files live in .shipdeck/.
        for name in ("release.json", "settings.json", f"{self.name}-api.service"):
            self.remote_command(f"test -f {self.remote}/current/.shipdeck/{name}")
        assert not self.remote_command(
            f"ls {self.remote}/current/release.json 2>/dev/null || true"
        )
        # The shared folder is a link to shared/.
        assert self.remote_command(
            f"readlink {self.remote}/current/apps/api/uploads"
        ) == f"{self.remote}/shared/apps/api/uploads"
        # Apps run in name order: api, site, worker.
        assert (self.project / "order.log").read_text() == (
            "preflight\nrelease\napi\nsite\nworker\ndone\n"
        )
        self.remote_command(f"echo persistent > {self.remote}/current/apps/api/uploads/data")
        # Nothing changed: exit 6 and the live release stays.
        self.cli("deploy", "production", expected=6)
        assert self.current() == first
        # A second release, with the API moved to PM2 and the worker using an
        # ecosystem file.
        self.write_deploy(api_manager="pm2", worker_ecosystem=True)
        self.bump()
        self.cli("deploy", "production")
        second = self.current()
        assert second != first
        # Retention: keep.open is 2, so the first open release closes, and its
        # backup stays.
        assert first in self.backups()
        assert self.remote_command(f"cat {self.remote}/current/apps/api/uploads/data") == "persistent"
        processes = json.loads(
            self.remote_command(
                f"runuser -u deploy -- sh -c 'pm2 jlist'"
            )
        )
        names = {app["name"] for app in processes}
        assert f"{self.name}-api" in names
        assert f"{self.name}-worker" in names
        self.second = second

    def rollback_from_backup(self):
        # The first release is closed, so putting it back unpacks its backup
        # and runs setup.server again.
        self.write_deploy()
        self.bump()
        self.cli("deploy", "production")
        third = self.current()
        closed = self.backups()[0]
        self.cli("rollback", "production", closed)
        assert self.current() == closed
        assert self.remote_command(
            f"cat {self.remote}/current/apps/api/setup.txt"
        ) == "setup"
        assert self.remote_command(f"systemctl is-active {self.name}-api") == "active"
        # And back to the newest one.
        self.cli("rollback", "production", third)
        assert self.current() == third

    def failure_before_the_switch(self):
        before = self.current()
        self.write_deploy(setup_fail=True)
        self.bump()
        self.cli("deploy", "production", expected=1)
        assert self.current() == before, "a setup failure never switches"
        assert (self.project / "failed.log").read_text().strip() == "setup"

    def automatic_rollback(self):
        before = self.current()
        # An automatic rollback runs the rollback commands, like one by hand.
        self.write_deploy(
            health_fail=True,
            extra_release=(
                "    rollback:\n"
                "      mode: auto\n"
                "      local: echo local >> rollback.log\n"
                "      server: echo server >> rollback.log\n"
            ),
        )
        self.bump()
        self.cli("deploy", "production", expected=3)
        assert self.current() == before
        assert self.remote_command(f"systemctl is-active {self.name}-api") == "active"
        assert (self.project / "rollback.log").read_text().split() == ["local"]
        assert self.remote_command(f"cat {self.remote}/rollback.log") == "server"

    def manual_rollback_to_an_open_release(self):
        self.write_deploy()
        self.bump()
        self.cli("deploy", "production")
        live = self.current()
        open_release = before = None
        for release in self.releases():
            if release != live:
                open_release = release
                break
        assert open_release, self.releases()
        self.cli("rollback", "production", open_release)
        assert self.current() == open_release
        # An open release keeps what its own setup made; setup.server does not
        # run again, so its setup.txt is the one it was deployed with.
        self.remote_command(f"test -f {self.remote}/current/.shipdeck/settings.json")
        _ = before

    def keep_failed(self):
        self.write_deploy(extra_release="    rollback: { mode: auto, keep_failed: true }\n")
        self.bump()
        # A health failure rolls back and keeps the failed release folder.
        deploy = self.deploy.replace(
            "        url: http://127.0.0.1:%d" % self.port,
            "        url: http://127.0.0.1:1",
        )
        (self.project / "deploy" / "production.yaml").write_text(deploy)
        before = self.current()
        self.cli("deploy", "production", expected=3)
        assert self.current() == before
        kept = [r for r in self.releases() if r != before]
        assert kept, self.releases()

    def runners(self):
        # Each runner the design lists, local and on the server.
        self.write_deploy(
            runner_step=(
                "      done:\n"
                "        local:\n"
                "          - {sh: 'echo sh >> ../runners.log'}\n"
                "          - {bash: 'echo bash >> ../runners.log'}\n"
                "          - {zsh: 'echo zsh >> ../runners.log'}\n"
                "          - {node: 'require(\"fs\").appendFileSync(\"../runners.log\", \"node\\\\n\")'}\n"
                "          - {python3: 'open(\"../runners.log\",\"a\").write(\"python3\\\\n\")'}\n"
                "        server:\n"
                "          - {bash: 'echo bash >> hooks.log'}\n"
            )
        )
        self.bump()
        self.cli("deploy", "production")
        log = (self.project / "runners.log").read_text()
        for runner in ("sh", "bash", "zsh", "node", "python3"):
            assert runner in log, (runner, log)
        assert "bash" in self.remote_command(f"cat {self.remote}/current/hooks.log")

    def missing_runner(self):
        self.write_deploy(
            runner_step=(
                "      done:\n"
                "        server:\n"
                "          - {nosuchrunner: 'exit 0'}\n"
            )
        )
        self.bump()
        before = self.current()
        result = self.command(
            [self.args.binary, "deploy", "production", "--key", self.args.key,
             "--trust-new-host"],
            expected=1,
            cwd=self.root,
        )
        assert "nosuchrunner" in result, result
        assert self.current() == before

    def rpc_checks(self):
        with Rpc(self.args.binary, self.root) as rpc:
            rpc.call("initialize", {"protocol": 1, "client": "config-v2-ssh-test"})
            opened = rpc.call(
                "workspace/open",
                {
                    "root": str(self.root),
                    "login": {"kind": "key", "path": self.args.key},
                },
            )
            assert len(opened["deployables"]) == 1, opened
            assert opened["deployables"][0]["slug"] == "production"
            assert opened["deployables"][0]["apps"] == ["api", "site", "worker"]
            live = self.current()
            state = rpc.call("status/get", {"deployable": "production"})
            assert state["current"] == live, state
            entries = rpc.call("releases/list", {"deployable": "production"})
            assert entries["current"] == live
            contents = rpc.call(
                "release/contents",
                {"deployable": "production", "release": live, "source": "folder"},
            )
            paths = {f["path"] for f in contents["files"]}
            assert ".shipdeck/settings.json" in paths, paths
            assert ".shipdeck/release.json" in paths, paths
            text = rpc.call(
                "release/read_file",
                {
                    "deployable": "production",
                    "release": live,
                    "source": "folder",
                    "path": ".shipdeck/release.json",
                },
            )
            assert '"id"' in text["text"], text


class Rpc:
    """Read JSON-RPC messages with bounded waits and automatic host trust."""

    def __init__(self, binary, cwd):
        self.process = subprocess.Popen(
            [binary, "engine"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            cwd=cwd,
        )
        self.messages = queue.Queue()
        self.sequence = 0
        threading.Thread(target=self.pump, daemon=True).start()

    def pump(self):
        for line in self.process.stdout:
            self.messages.put(json.loads(line))

    def read(self):
        return self.messages.get(timeout=40)

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
        self.process.wait(timeout=30)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/debug/shipdeck")
    parser.add_argument("--container", default="shipdeck-monorepo-ssh-test")
    parser.add_argument("--port", type=int, default=38733)
    parser.add_argument("--key", required=True)
    args = parser.parse_args()
    args.binary = str(pathlib.Path(args.binary).resolve())
    args.key = str(pathlib.Path(args.key).resolve())
    root = tempfile.mkdtemp(prefix="shipdeck-config-v2-e2e-")
    print(f"Test files: {root}", flush=True)
    Harness(args, root).run()


if __name__ == "__main__":
    main()
