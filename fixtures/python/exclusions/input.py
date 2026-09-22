import os


def outer():
    def inner():
        ...

    class Local:
        ...

    return lambda value: value


class Host:
    def method(self):
        ...
