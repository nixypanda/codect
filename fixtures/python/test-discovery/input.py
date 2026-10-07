"""Module docstring."""
import unittest

from typing import TypeAlias

Alias: TypeAlias = "int"

MAX: int = 10


def helper(value: int) -> int:
    return value


def test_a_free_function() -> None:
    assert helper(1) == 1


async def test_an_async_test() -> None:
    await nothing()


def test_wrapped_in_parens() -> bool:
    return True


def not_a_test() -> None:
    ...


@pytest.mark.parametrize("value", [1, 2])
def test_a_decorated_test(value: int) -> None:
    assert value


@pytest.fixture()
def a_fixture() -> int:
    return 1


class NotATest:
    def method(self) -> None:
        ...

    value: int


class TestThing:
    attribute: int

    def test_first(self) -> None:
        ...

    def test_second(self) -> int:
        return 1

    def helper(self) -> None:
        ...

    value: int


class TestOther(unittest.TestCase):
    def test_case_method(self) -> None:
        ...

    def setUp(self) -> None:
        ...


class TestEmpty:
    def helper_only(self) -> None:
        ...