# FetchX Plugins

This directory holds installed FetchX plugins. Each plugin lives in its own subdirectory named by the plugin ID (e.g. `dev.fetchx.drm/`).

## What are plugins?

Plugins extend FetchX with optional features that cannot be included in the base app (for example, DRM decryption). They run as isolated processes and communicate with FetchX over a sandboxed IPC channel.

## Installing a plugin

### From a GitHub release (recommended)
In FetchX, open **Settings → Plugins → Install from GitHub** and enter the repository slug, e.g.:
```
MeherMankar/fetchx-drm-plugin
```
FetchX downloads the correct binary for your platform and installs it automatically.

### Manual installation
1. Download the plugin release archive for your platform.
2. Extract it into a folder named by the plugin ID inside this `plugins/` directory.
   Example: `plugins/dev.fetchx.drm/`
3. The folder must contain `plugin.json` and the plugin executable.
4. Restart FetchX — it will detect and register the plugin automatically.

## Available plugins

| Plugin | Description | Repo |
|--------|-------------|------|
| FetchX DRM (`dev.fetchx.drm`) | Widevine & PlayReady DRM decryption | [MeherMankar/fetchx-drm-plugin](https://github.com/MeherMankar/fetchx-drm-plugin) |

## Plugin security

FetchX validates each plugin's `plugin.json` manifest and runs the plugin as a separate process with limited permissions. Always install plugins from sources you trust.
