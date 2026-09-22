"""Module documentation is omitted."""
from __future__ import annotations

from typing import Generic, TypeVar

T = TypeVar("T")

type UserId = int


class Profile(Generic[T]):
    name: str
    email: str


class Status(Enum):
    ACTIVE = auto()
    SUSPENDED = auto()
