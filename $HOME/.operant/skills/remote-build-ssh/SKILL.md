---
name: remote-build-ssh
description: Build Rust cargo binaries remotely on the ishanp build machine via SSH through a cloudflared tunnel. Use this skill whenever you need to compile a Rust project on a powerful remote machine, when the user says "build on the remote box", "compile remotely", "SSH and build", "deploy to build server", or any time you need to push code and produce binaries on the ishanp build server. Works for ANY Rust project — not just operant. Also use when setting up a new AI agent to access this build machine, or when troubleshooting SSH tunnel connectivity.
metadata:
  operant: {}
---

# Remote Rust Build via SSH

This skill enables AI agents to SSH into the build machine through a Cloudflare tunnel, clone/pull any Rust repo, and compile cargo binaries. Each SSH command is fully self-contained — AI agents cannot maintain interactive SSH sessions, so every operation is a single `ssh ... 'command'` call.

## Prerequisites

**cloudflared must be installed on the client machine** (VPS, laptop, or wherever the agent runs). This is required for the SSH tunnel to work.

```bash
# Check if cloudflared is installed
which cloudflared || echo "NOT INSTALLED"

# Install if missing (Linux amd64)
curl -L https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64 -o /usr/local/bin/cloudflared && chmod +x /usr/local/bin/cloudflared
```

## Build Machine Specs

| Resource | Value |
|----------|-------|
| Host | `home-ssh.ishanparihar.com` |
| User | `ishanp` |
| OS | Linux (x86_64, CachyOS/Arch) |
| CPU | 24 cores |
| RAM | 46 GB |
| Disk | ~120 GB free |
| Rust | 1.94.1 (rustup) |
| Cargo | 1.94.1 |

## SSH Access Pattern (Critical)

**AI agents cannot interact within SSH sessions.** Every command must be a self-contained SSH call.

**Required: Always use ProxyCommand with cloudflared access.** Direct SSH to `home-ssh.ishanparihar.com` will fail with "Cannot assign requested address" because the hostname resolves via Cloudflare Tunnel, not DNS.

```bash
# CORRECT — works every time:
ssh -o ProxyCommand="cloudflared access ssh --hostname home-ssh.ishanparihar.com" -o StrictHostKeyChecking=accept-new ishanp@home-ssh.ishanparihar.com 'YOUR_COMMAND_HERE'

# WRONG — will fail:
ssh ishanp@home-ssh.ishanparihar.com 'YOUR_COMMAND_HERE'
```

For multi-step operations, chain with `&&`:

```bash
ssh -o ProxyCommand="cloudflared access ssh --hostname home-ssh.ishanparihar.com" -o StrictHostKeyChecking=accept-new ishanp@home-ssh.ishanparihar.com 'cd /path/to/project && git pull && cargo build --release'
```

Or use a heredoc-style approach for complex scripts:

```bash
ssh -o ProxyCommand="cloudflared access ssh --hostname home-ssh.ishanparihar.com" -o StrictHostKeyChecking=accept-new ishanp@home-ssh.ishanparihar.com 'bash -s' << 'REMOTE_SCRIPT'
cd /home/ishanp/Documents/GitHub/MY-PROJECTS/your-project
git pull --ff-only
source scripts/setup-build-env.sh 2>/dev/null || export LIBCLANG_PATH=/usr/lib CARGO_INCREMENTAL=0
cargo build --release
REMOTE_SCRIPT
```

## Setup for VPS (RackNerd)

The RackNerd VPS (`nerd@107.173.144.77:25897`) has cloudflared installed. From the VPS:

```bash
# Install cloudflared if not present
which cloudflared || (curl -L https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64 -o /usr/local/bin/cloudflared && chmod +x /usr/local/bin/cloudflared)

# Test connectivity
ssh -o ProxyCommand="cloudflared access ssh --hostname home-ssh.ishanparihar.com" -o StrictHostKeyChecking=accept-new ishanp@home-ssh.ishanparihar.com 'echo TUNNEL_OK && hostname'

# Add to ~/.ssh/config for convenience
cat >> ~/.ssh/config << 'EOF'
Host build-machine
    HostName home-ssh.ishanparihar.com
    User ishanp
    ProxyCommand cloudflared access ssh --hostname %h
    StrictHostKeyChecking accept-new
EOF

# Now you can just use:
ssh build-machine 'echo TUNNEL_OK'
```

## Generic Rust Project Workflow

For ANY Rust project on the build machine:

### Option 1: One-shot build (recommended)

```bash
ssh -o ProxyCommand="cloudflared access ssh --hostname home-ssh.ishanparihar.com" -o StrictHostKeyChecking=accept-new ishanp@home-ssh.ishanparihar.com 'cd /home/ishanp/Documents/GitHub/MY-PROJECTS/your-project && git pull --ff-only && source scripts/setup-build-env.sh 2>/dev/null || export LIBCLANG_PATH=/usr/lib CARGO_INCREMENTAL=0 && cargo build --release 2>&1'
```

### Option 2: Clone and build a new project

```bash
ssh -o ProxyCommand="cloudflared access ssh --hostname home-ssh.ishanparihar.com" -o StrictHostKeyChecking=accept-new ishanp@home-ssh.ishanparihar.com 'cd /home/ishanp/Documents/GitHub/MY-PROJECTS && git clone https://github.com/your-org/your-project.git && cd your-project && cargo build --release 2>&1'
```

### Option 3: Build specific package only (faster)

```bash
ssh -o ProxyCommand="cloudflared access ssh --hostname home-ssh.ishanparihar.com" -o StrictHostKeyChecking=accept-new ishanp@home-ssh.ishanparihar.com 'cd /home/ishanp/Documents/GitHub/MY-PROJECTS/your-project && git pull --ff-only && cargo build --release -p your-package 2>&1'
```

### Option 4: Check only (fastest, no binary produced)

```bash
ssh -o ProxyCommand="cloudflared access ssh --hostname home-ssh.ishanparihar.com" -o StrictHostKeyChecking=accept-new ishanp@home-ssh.ishanparihar.com 'cd /home/ishanp/Documents/GitHub/MY-PROJECTS/your-project && cargo check --workspace 2>&1'
```

### Option 5: Run tests

```bash
ssh -o ProxyCommand="cloudflared access ssh --hostname home-ssh.ishanparihar.com" -o StrictHostKeyChecking=accept-new ishanp@home-ssh.ishanparihar.com 'cd /home/ishanp/Documents/GitHub/MY-PROJECTS/your-project && cargo test --workspace 2>&1'
```

## Retrieving Built Binaries

After a successful build, copy the binary back to your machine:

```bash
# Via scp (note: scp also needs ProxyCommand)
scp -o ProxyCommand="cloudflared access ssh --hostname home-ssh.ishanparihar.com" -o StrictHostKeyChecking=accept-new ishanp@home-ssh.ishanparihar.com:/home/ishanp/Documents/GitHub/MY-PROJECTS/your-project/target/release/your-binary .

# Or use ssh to cat the binary (for smaller binaries)
ssh -o ProxyCommand="cloudflared access ssh --hostname home-ssh.ishanparihar.com" -o StrictHostKeyChecking=accept-new ishanp@home-ssh.ishanparihar.com 'cat /home/ishanp/Documents/GitHub/MY-PROJECTS/your-project/target/release/your-binary' > your-binary
chmod +x your-binary
```

## Environment Variables

The build machine requires these environment variables for compilation:

```bash
export LIBCLANG_PATH=/usr/lib
export CARGO_INCREMENTAL=0
export BINDGEN_EXTRA_CLANG_ARGS="-I/usr/lib/gcc/x86_64-linux-gnu/$(gcc -dumpversion)/include -I/usr/include"
```

For projects using ONNX Runtime (kokoro-tts):
```bash
export ORT_LIB_LOCATION=/home/ishanp/Documents/GitHub/MY-PROJECTS/local/onnxruntime-linux-x64-1.20.1/lib
export ORT_PREFER_DYNAMIC_LINK=1
```

Use the build script if available:
```bash
source scripts/setup-build-env.sh
```

## Troubleshooting

### SSH connection fails
```bash
# Verify cloudflared is installed
which cloudflared

# Test cloudflared access directly
cloudflared access ssh --hostname home-ssh.ishanparihar.com

# Check if the tunnel is up (from the build machine if you have other access)
systemctl status cloudflared
```

### Build fails with libclang error
```bash
# Find libclang on the build machine
ssh build-machine 'find /usr -name "libclang*.so*" 2>/dev/null'

# Set the path manually
ssh build-machine 'export LIBCLANG_PATH=/usr/lib && cargo build --release'
```

### Disk space issues
```bash
# Check disk usage
ssh build-machine 'df -h / && du -sh ~/Documents/GitHub/MY-PROJECTS/*/target 2>/dev/null | sort -rh | head -5'

# Clean old build artifacts
ssh build-machine 'cargo clean --release'
```

### Build is slow
```bash
# Use scoped build (faster)
ssh build-machine 'cd /path/to/project && cargo build --release -p specific-package'

# Or just check (fastest)
ssh build-machine 'cd /path/to/project && cargo check --workspace'
```

## Notes for AI Agents

1. **Always use ProxyCommand** — direct SSH will fail
2. **Self-contained commands only** — no interactive sessions
3. **Chain with &&** for multi-step operations
4. **Use scripts/setup-build-env.sh** if it exists in the project
5. **For operant specifically**: `scripts/build-operant.sh` handles everything
6. **Binary path**: `target/release/your-binary-name`
7. **Copy binaries back** with `scp` (also needs ProxyCommand)

## Stale Binary Detection (Critical)

When a config file references fields that exist in the current **source code**
but the **deployed binary** rejects them with `unknown field` errors, the deployed
binary is stale. Always check the debug binary first:

```bash
# The debug binary at target/debug/ is often built from current source
# and may accept config fields the deployed binary does not.
./target/debug/operant channel list -c /path/to/config.toml

# If the debug binary works, rebuild and deploy:
source scripts/dev-env.sh
cargo build --release -p operant-cli
sudo cp target/release/operant /usr/local/bin/operant  # or ~/.cargo/bin/operant
operant --version  # confirm version matches expectations
```

If `which operant` resolves to a stale binary (e.g. `~/.cargo/bin/operant` v0.1.4
that lacks LCM fields added in source), either install the freshly-built binary
to the correct location or use the full path to `target/debug/operant`.
