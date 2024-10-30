# good-fences-rs-core

<!-- Core implementation of [`good-fences-rs`](https://github.com/Adjective-Object/good-fences.rs) -->


<!-- Written against `rustc 1.47.0 (18bf6b4f0 2020-10-07)` -->

A rust reimplementation of [good-fences](https://github.com/smikula/good-fences):
> Good-fences is a tool that allows you to segment a TypeScript project into conceptual areas and manage dependencies between those areas.
This is mostly a concern for large projects with many people working on them, where it is impossible for every developer to have a wholistic understanding of the entire codebase. JavaScript's module system is a specialized form of anarchy because any file can import any other file, possibly allowing access to code that was really meant to be an internal implementation detail of some larger system. Other languages have concepts like DLL boundaries and the internal keyword to mitigate this. Good-fences provides a way to enforce similar boundaries in the TypeScript world.

## Motivation

The original good-fences implementation came with some limitations:
- Its native dependencies only supported Node.js < v15.
- It had performance issues in some of our biggest projects (scanning 40k+ files).

Rust's safe concurrency and memory safety allows us to re-write original project with additional performance benefits, leaning on [swc](https://github.com/swc-project/swc/) for javascript/typescript parsing.
## Getting Started

`good-fences-rs` includes a CLI and an API, under the name `@good-fences/api`.

# Requirements

Compatible with `x86` and `x64` windows and linux platforms.

_Linux_:
- `GCLIB` >= 2.27 (preinstalled with ubuntu 18)
- Node.js > 14
- `npm`


# Installing CLI

Via npm.

``` sh
npm install -g @good-fences/api
```

Cloning the repo:

``` sh
git clone https://github.com/Adjective-Object/good-fences-rs-core
cd good-fences-rs-core
yarn
yarn run build
```

# Installing as Node dependency

``` sh
npm install @good-fences/api
```

Use it in your project:
``` js
import { goodFences } from '@good-fences/api';

goodFences({...});
```

# Using the CLI

To run the `good-fences` cli we need at least two things:
- `fence.json` configuration files.
- A `tsconfig.json` file. (see [tsconfig reference](https://www.typescriptlang.org/tsconfig))

Let's assume a project like this:

```
├── my-project
│   ├── src
│   │   ├── **/*.ts
|   |   ├── index.js
|   │   ├── fence.json
|   tsconfig.json
```

From your terminal you can run this:
``` sh
cd my-project
good-fences src
```

## Arguments
---

- `[paths]`: the cli takes only the `paths` argument, a list, separated with spaces, of all directories that are going to be scanned.

## Options
---
### `--project` or `-p`

If you have your tsconfig file splitt and want to use the one containing `compilerOptions.paths` instead of the default `tsconfig.json`
``` sh
good-fences src --project tsconfig.with-paths.json
```
### `--baseUrl`

In cases like the one above, it could be that different tsconfig files have different `compilerOptions.baseUrl` configuration, you can override that valua from your specified `--project` file with `--baseUrl` flag.

``` sh
good-fences src --project tsconfig.without-baseurl.json --baseUrl .
```

### `--output` or `-o`
The `--output` flag takes a path. At the end of checking, fence violation errors will be saved to the provided path as json.

``` sh
good-fences src --output fenceViolations.json
cat fenceViolations.json
```

For some cases, scanning your `cwd` could be needed but most projects have `node_modules` that isn't necessary to perform evaluations, in those cases `--ignoreExternalFences` makes good-fences skip all directories and files from `node_modules`.
``` sh
good-fences . --ignoreExternalFences
```

### `--ignoredDirs`
This takes a list of regular expressions as input values, separated with spaces. In case certain directories need to be ignored during the fence evaluation, this will perform regular expression matching on fence paths to ignore them (e.g. `--ignoredDirs lib` will not evaluate files under any `lib` directory).

``` sh
good-fences src --ignoredDirs ignored1 ignored2 ...
```

# Development

## Setting up the Development Environment
1. Install a container engine:
    The repo uses a devcontainer, which is like a lightweight virtual machine that contains a pre-configured development environment.
    
    It is intended to support both Docker and podman, which are two different container engines. This is kind of arbitrary, and I might choose to revert it in the future if it presents issues.

    On windows, install Docker-Desktop
    On linux, you can install either `docker` or podman (via `podman-docker`)

2. Set up your local config
    The devcontainer expects several user directories to be configured. If you do not have these already, you will have to create them, or comment out the bind mounts in the devcontainer.

    - `$HOME/.gitconfig` -- This is mounted to make your git configuration available in the container.  
        It should already exist if you've ever run `git config --user.name` or `git config --user.email`
    - `$HOME/.ssh` -- This is mounted so the container can access your SSH keys to push/pull from the git remote.  
        This should already exist if you have ever configured an SSH key via `ssh-keygen`, which is the normal way to clone a git repo.

    Note that if you are developing in WSL, you should create these files _in wsl_, not within your windows filesystem.
3. Install recommended extensions
    Install the recommended extensions from this repo.
    
    `Ctrl+Shift+P > Extensions: Show Recommended Extensions`, then install all recommended extensions from the left navbar that opens up.
4. Build and open in the devcontainer

    `Ctrl+Shift+P > Rebuild and Reopen In Container`
    Select the development container based on your container engine (podman or docker)

    - If installation stalls on `docker inspect --type image ubuntu:24.10`, you may need to feth the base image manually
    - Run `docker inspect --type image ubuntu:24.10`
    - If it fails with `Error response from daemon: No such image: ubuntu`, then run `docker pull ubuntu:24.10`
5. (optional) mount additional projects into the dev container
   To test `unused-finder` against your repo during development, uncomment the commented-out "mount" in the checked-in `.devcontainer`: 
   ```json5
    // This mounts client-web checked out next to this repo for testing, left checked-in here for convenience.
    // Don't commit it, though!
    // "source=${localWorkspaceFolder}/../client-web,target=/workspaces/client-web,type=bind,consistency=cached",
    ```
    The provided example mounts `client-web` as a target repo

## Flamegraphs and profiling
For profiling, you can use [`samply`](https://github.com/mstange/samply)
```sh
# This isn't installed by default in the dev container because it has to be built from source,
# which takes a long time
#
# Must be built with --locked dependencies
# See: https://github.com/mstange/samply/issues/341
cargo install samply --locked
```

To profile a test, first build the test binary
```sh
# This will print the path of the test binary
cargo test -p unused_finder --no-run

# Then, run samply on the test binary binary
samply record target/debug/deps/unused_finder-3aa70b00191bd4df
```

### Note: Working in WSL
The devcontainer is configured to allow perf events, but the host system must also be configured to allow perf events. On windows, devcontainers will probably be running under wsl. This means opening up wsl, and running the following:
```sh
# from within WSL
echo '1' | sudo tee /proc/sys/kernel/perf_event_paranoid
```
Then, close and restart your devcontainers