# docker-sandbox-tui

A `create-vite-app`-like TUI tool for managing Docker sandboxes for agents with ease. I've vibecoded it because Docker sandbox UX sucks.

## Features

- **Interactive Wizard**: Step-by-step prompts to create new sandboxes with custom agents, workspaces, names, and Docker images.
- **Quick Launch**: Use `agent .` for zero-interaction fast path to resume or create sandboxes with config-specified defaults.
- **Configuration**: Save preferences (default agent and VS Code behavior) to a YAML config file (`~/.config/docker-sandbox-tui/config.yaml`).
- **Resume Existing Sandboxes**: Automatically detects and offers to resume sandboxes for the current directory.
- **Agent Selection**: Choose from available agents interactively
- **Template Support**: Optionally use local Docker images or specify custom image names/URLs (not fully tested)
- **VS Code Integration**: (Configurable) Automatically open VS Code in the workspace after sandbox creation.

## Installation

### Prerequisites

- Rust (latest stable version recommended)
- Docker installed and running
- (Optional) VS Code for integration

### Build from Source

1. Clone or download the repository.
2. Navigate to the project directory.
3. Run `cargo build --release` to build the binary.
4. The executable `agent` will be in `target/release/`.


## Usage

### Basic Usage

Run the binary:

```bash
./agent
```

This starts the interactive wizard.

### Quick Launch

For fast launching in the current directory:

```bash
./agent .
```

This resumes an existing sandbox or creates a new one with default settings.

### Command Line Arguments (optional)

- Provide a path as an argument to specify the workspace directory.
- Use `./` for current directory, `.` would create a sandbox with config defined defaults

### Configuration

Configuration is stored in `~/.config/docker-sandbox-tui/config.yaml` (or equivalent on your OS).

Options:
- `agent`: Default agent (e.g., "claude")
- `open_vscode`: Whether to open VS Code by default (true/false)
- `show_vscode_prompt`: Whether to show the VS Code prompt (true/false)

The tool will prompt to save new defaults after the first interactive run.
