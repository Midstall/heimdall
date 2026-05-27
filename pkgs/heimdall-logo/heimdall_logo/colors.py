"""Heimdall logo color palette. Hardcoded Tokyo Night accent stops so the
Heimdall repo doesn't have to pull in Midstall's color-palette TOML at
build time. The two gradient endpoints match the Web UI's --accent-purple
and --accent-blue-light in crates/heimdall-daemon/assets/app.css."""


class HeimdallPalette:
    # Backgrounds
    bg_night = "#1a1b26"
    bg_dark = "#16161e"

    # Gradient endpoints used on the logomark stroke + halo.
    accent_start = "#bb9af7"  # purple
    accent_end = "#7dcfff"    # cyan / light blue

    # Foreground text colors.
    fg_bright = "#c0caf5"
    fg_primary = "#a9b1d6"

    def gradient_stops(self) -> list[tuple[float, str]]:
        return [(0.0, self.accent_start), (1.0, self.accent_end)]
