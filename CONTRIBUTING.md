# Contributing

## Tests

```sh
cargo test          # macOS or Linux, against real files and processes
ci/linux.sh         # the same suite on Linux in Docker, no network, unprivileged user
```

The tests use no mocks. They create real files and directories in the system temporary directory,
change their permissions, and run the `glassbox` binary against a temporary home directory. The
permission test is skipped when run as root, because root bypasses permission bits; `ci/linux.sh`
runs as an unprivileged user so it is not skipped there.

## Contributing

Changes reach `main` only through pull requests.

1. Create a branch.
2. Make the change with a test that fails without it, then run `cargo test` and `ci/linux.sh`.
3. Open a pull request against `main`. Pull requests are merged with a merge commit.

Keep glassbox dependency-free, never read the contents of what it probes, and never write.
