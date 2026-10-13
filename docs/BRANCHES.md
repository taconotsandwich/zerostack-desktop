# Development branches

The shared engine refactor is based on ACP. Frontend and SDK work are sibling
branches above that foundation:

```text
refactor/acp-engine
└── refactor/engine-core
    ├── feature/sdk
    ├── redesign/tui-ux
    └── feature/desktop
```

`refactor/engine-core` owns shared execution, workspace isolation, model
operations, cancellation and shutdown. It retains the existing CLI/TUI as
an adapter so the foundation can be compiled and tested independently.

`feature/sdk` starts at that foundation; the Rust embedding API is still
future work. `redesign/tui-ux` owns the command palette, unified model picker
and model alias saving flow. `feature/desktop` carries the desktop frontend.
Neither frontend depends on the other frontend's branch.

Put shared fixes on the engine branch and propagate them to each child.
Before rewriting a child branch, preserve its current tip; coordinate any
published history changes separately. See [ENGINE.md](ENGINE.md) for current
engine boundaries and the remaining work before publishing a library.
