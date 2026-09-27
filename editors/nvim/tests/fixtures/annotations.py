#!/usr/bin/env python
"""Module docstring."""


def first():
    return 1
# trailing comment that belongs to first, not to second
def second():
    return 2


@app.route(
    "/health",
    methods=["GET"],
)
def health():
    return {}


@staticmethod
def simple():
    return 1
