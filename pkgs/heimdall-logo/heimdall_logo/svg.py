"""Minimal SVG builder. Same shape as the Midstall branding helper but
vendored here so the Heimdall repo doesn't depend on the branding repo."""

from xml.etree.ElementTree import Element, SubElement, tostring


class SvgDocument:
    """Builds an SVG document from elements."""

    def __init__(
        self,
        width: float,
        height: float,
        viewbox: tuple[float, float, float, float] | None = None,
    ):
        self.root = Element("svg", xmlns="http://www.w3.org/2000/svg")
        self.root.set("width", str(width))
        self.root.set("height", str(height))
        vb = viewbox or (0, 0, width, height)
        self.root.set("viewBox", f"{vb[0]} {vb[1]} {vb[2]} {vb[3]}")
        self.defs = SubElement(self.root, "defs")

    def group(self, parent: Element | None = None, **attrs: str) -> Element:
        p = parent if parent is not None else self.root
        g = SubElement(p, "g")
        _apply(g, attrs)
        return g

    def line(
        self, parent: Element, x1: float, y1: float, x2: float, y2: float, **attrs: str
    ) -> Element:
        el = SubElement(parent, "line")
        el.set("x1", _fmt(x1))
        el.set("y1", _fmt(y1))
        el.set("x2", _fmt(x2))
        el.set("y2", _fmt(y2))
        _apply(el, attrs)
        return el

    def rect(
        self, parent: Element, x: float, y: float, w: float, h: float, **attrs: str
    ) -> Element:
        el = SubElement(parent, "rect")
        el.set("x", _fmt(x))
        el.set("y", _fmt(y))
        el.set("width", _fmt(w))
        el.set("height", _fmt(h))
        _apply(el, attrs)
        return el

    def circle(
        self, parent: Element, cx: float, cy: float, r: float, **attrs: str
    ) -> Element:
        el = SubElement(parent, "circle")
        el.set("cx", _fmt(cx))
        el.set("cy", _fmt(cy))
        el.set("r", _fmt(r))
        _apply(el, attrs)
        return el

    def path(self, parent: Element, d: str, **attrs: str) -> Element:
        el = SubElement(parent, "path")
        el.set("d", d)
        _apply(el, attrs)
        return el

    def linear_gradient(
        self,
        gid: str,
        stops: list[tuple[float, str]],
        x1: float = 0,
        y1: float = 0,
        x2: float = 1,
        y2: float = 1,
        units: str = "userSpaceOnUse",
    ) -> Element:
        """Add a linearGradient to <defs>. `stops` is (offset 0..1, color).

        `units` defaults to `userSpaceOnUse` because the default
        `objectBoundingBox` mode collapses to a single color on degenerate
        elements (vertical lines, horizontal lines).
        """
        grad = SubElement(self.defs, "linearGradient")
        grad.set("id", gid)
        grad.set("gradientUnits", units)
        grad.set("x1", _fmt(x1))
        grad.set("y1", _fmt(y1))
        grad.set("x2", _fmt(x2))
        grad.set("y2", _fmt(y2))
        for offset, color in stops:
            s = SubElement(grad, "stop")
            s.set("offset", f"{offset * 100:.1f}%")
            s.set("stop-color", color)
        return grad

    def to_string(self) -> str:
        return '<?xml version="1.0" encoding="UTF-8"?>\n' + tostring(
            self.root, encoding="unicode"
        )

    def write(self, path: str) -> None:
        with open(path, "w") as f:
            f.write(self.to_string())


def _apply(el: Element, attrs: dict[str, str]) -> None:
    for k, v in attrs.items():
        el.set(k.replace("_", "-"), str(v))


def _fmt(n: float) -> str:
    """Format a number without unnecessary precision."""
    if isinstance(n, int):
        return str(n)
    return f"{n:.4f}".rstrip("0").rstrip(".")
