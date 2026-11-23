/// Which install surfaces `atlas install` should set up.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub enum InstallMode {
    /// Git hooks and the generic Atlas agent hook runner only.
    Hook,
    /// Managed `AGENTS.md` instruction block only.
    Cli,
    /// Both hook and instruction surfaces (default).
    #[default]
    All,
}

impl InstallMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hook => "hook",
            Self::Cli => "cli",
            Self::All => "all",
        }
    }
}

/// Shell-first install surfaces written by `atlas init --mode` (ICM-H1).
///
/// `mcp` is the default so a plain `atlas init` stays non-invasive: it writes a
/// copyable stdio server entry under `.atlas/agent/` and never touches tracked
/// repository files. Hooks and `AGENTS.md` instructions require an explicit
/// `hook`, `cli`, or `all` mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub enum InitMode {
    /// Generic MCP server registration asset only (default).
    #[default]
    Mcp,
    /// Git hooks and the tool-agnostic Atlas agent hook runner only.
    Hook,
    /// Managed `AGENTS.md` CLI/agent instruction block only.
    Cli,
    /// MCP asset, hooks, and CLI instructions.
    All,
}

impl InitMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mcp => "mcp",
            Self::Hook => "hook",
            Self::Cli => "cli",
            Self::All => "all",
        }
    }

    /// Whether this mode writes the generic MCP registration asset.
    pub const fn includes_mcp(self) -> bool {
        matches!(self, Self::Mcp | Self::All)
    }

    /// Whether this mode installs git hooks and the hook runner.
    pub const fn includes_hooks(self) -> bool {
        matches!(self, Self::Hook | Self::All)
    }

    /// Whether this mode injects the managed `AGENTS.md` instruction block.
    pub const fn includes_cli(self) -> bool {
        matches!(self, Self::Cli | Self::All)
    }
}
