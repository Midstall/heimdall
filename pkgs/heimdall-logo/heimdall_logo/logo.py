"""Full Heimdall logo composition: logomark + 'HEIMDALL' wordmark below."""

from heimdall_logo.colors import HeimdallPalette
from heimdall_logo.logomark import Logomark
from heimdall_logo.logotype import FontLoader
from heimdall_logo.svg import SvgDocument


class Logo:
    """Composes the Heimdall logomark with the 'HEIMDALL' wordmark beneath
    it, matching the concept layout."""

    def __init__(
        self,
        logomark: Logomark | None = None,
        font_loader: FontLoader | None = None,
        font_size: float = 44,
        letter_spacing: float = 10,
        gap: float = 28,
        margin: float = 24,
        palette: HeimdallPalette | None = None,
    ):
        self.mark = logomark or Logomark()
        self.font = font_loader or FontLoader()
        self.font_size = font_size
        self.letter_spacing = letter_spacing
        self.gap = gap
        self.margin = margin
        self.palette = palette or HeimdallPalette()

    def render(self, background: str | None = None) -> SvgDocument:
        m = self.margin
        _, text_width, _ = self.font.text_to_paths(
            "HEIMDALL", self.font_size, self.letter_spacing
        )

        total_width = max(self.mark.size, text_width) + m * 2
        total_height = m + self.mark.size + self.gap + self.font_size + m

        doc = SvgDocument(total_width, total_height)
        doc.linear_gradient(
            "heimdall-grad",
            self.palette.gradient_stops(),
            x1=0, y1=0, x2=total_width, y2=total_height,
        )

        if background is not None:
            doc.rect(
                doc.root,
                0, 0, total_width, total_height,
                fill=background,
                rx=18, ry=18,
            )

        # Logomark centered horizontally.
        mark_x = (total_width - self.mark.size) / 2
        mark_group = doc.group(transform=f"translate({mark_x},{m})")
        self.mark.draw_into(doc, mark_group)

        # Wordmark centered beneath.
        text_x = (total_width - text_width) / 2
        text_y = m + self.mark.size + self.gap
        wordmark_group = doc.group()
        self.font.render_text_group(
            doc,
            wordmark_group,
            "HEIMDALL",
            text_x,
            text_y,
            self.font_size,
            self.palette.fg_bright,
            self.letter_spacing,
        )

        return doc
