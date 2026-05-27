"""CLI entry point. Used by the nix derivation to produce SVG artifacts:

    heimdall-logo logomark --output logomark.svg
    heimdall-logo favicon  --output favicon.svg
    heimdall-logo full     --output logo.svg --background "#1a1b26"
"""

import argparse
import sys

from heimdall_logo.colors import HeimdallPalette
from heimdall_logo.logomark import Logomark


def _cmd_logomark(args: argparse.Namespace) -> int:
    doc = Logomark().render(background=args.background, margin=args.margin)
    _write(doc, args.output)
    return 0


def _cmd_favicon(args: argparse.Namespace) -> int:
    # Favicon-friendly variant: drop the smallest details that would muddy
    # the shape at 32x32.
    mark = Logomark(
        size=256,
        wing_count=3,
        wing_top_y=0.18,
        trace_count=1,
        halo_arc_degrees=200,
    )
    doc = mark.render(background=args.background, margin=10)
    _write(doc, args.output)
    return 0


def _cmd_full(args: argparse.Namespace) -> int:
    # Lazy-import Logo because it touches fontforge and we want `logomark`
    # / `favicon` to work without the font being available.
    from heimdall_logo.logo import Logo

    doc = Logo().render(background=args.background)
    _write(doc, args.output)
    return 0


def _write(doc, output: str) -> None:
    if output == "-":
        sys.stdout.write(doc.to_string())
    else:
        doc.write(output)


def main(argv: list[str] | None = None) -> int:
    palette = HeimdallPalette()
    parser = argparse.ArgumentParser(prog="heimdall-logo")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p_mark = sub.add_parser("logomark", help="Render the symbol only.")
    p_mark.add_argument("--output", "-o", default="-", help="Output path or `-` for stdout.")
    p_mark.add_argument("--background", default=None, help="Optional background color.")
    p_mark.add_argument("--margin", type=float, default=12.0)
    p_mark.set_defaults(func=_cmd_logomark)

    p_fav = sub.add_parser("favicon", help="Logomark variant tuned for small sizes.")
    p_fav.add_argument("--output", "-o", default="-")
    p_fav.add_argument("--background", default=None)
    p_fav.set_defaults(func=_cmd_favicon)

    p_full = sub.add_parser("full", help="Logomark + HEIMDALL wordmark.")
    p_full.add_argument("--output", "-o", default="-")
    p_full.add_argument("--background", default=palette.bg_night)
    p_full.set_defaults(func=_cmd_full)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
