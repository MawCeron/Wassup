# Contributing

Thanks for wanting to help. Wassup is an unofficial WhatsApp client for
Linux, so test with a secondary number: Meta could restrict accounts that
use it.

## Reporting a problem

Open an [issue](https://github.com/MawCeron/Wassup/issues) and say what you
did, what you expected and what happened. Include the Wassup version and
your Fedora version. The daemon's log helps:
`journalctl --user -u wassupd`.

Do not paste private messages, names or phone numbers; describe them
instead.

## Setting up

[`docs/development.md`](docs/development.md) lists the packages, the commands
to build and test the daemon (Go) and the UI (Rust), and how the two talk
over the schema in `proto/ipc.proto`. The code is in `daemon/` and `ui/`;
[`docs/ipc.md`](docs/ipc.md) describes the protocol between them.

## Before sending a change

CI runs these on every pull request, and they must pass:

```sh
# daemon/
test -z "$(gofmt -l .)" && go vet ./... && go test ./...

# ui/
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

- If you change `proto/ipc.proto`, run `go generate ./...` in `daemon/` and
  commit `daemon/ipc/ipc.pb.go`.
- A new user-facing string goes through `gettext` in the UI or `tr` in the
  daemon; [`docs/translating.md`](docs/translating.md) explains how, and how
  to add or update a language.
- Add a test for logic you add or fix.
- Update the documentation in `docs/` when behavior changes, and add a line
  under `Unreleased` in [`CHANGELOG.md`](CHANGELOG.md) for what users will
  notice.

## Commits and pull requests

- Commit messages follow the style of the history: a type, a scope when it
  helps, and a short summary in the imperative, such as
  `fix(ui): keep following new messages` or `feat(daemon): send photos and
  documents`. The types in use are `feat`, `fix`, `docs`, `build`, `test`,
  `perf`, `refactor` and `chore`.
- Keep each commit to one concern.
- Write code, comments, commits and documentation in English.
- `main` takes changes through pull requests that pass CI.

## License

Wassup is under the [GNU GPL v3 or later](LICENSE). Your contribution is
licensed the same way.
