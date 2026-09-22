@dataclass(frozen=True)
class Point(Base, metaclass=Meta):
    """A point."""

    x: float
    y: float = 0.0


def area(width: int, *args, height: int = 1, **kwargs) -> int:
    return width * height
