import os.path as osp
from .helpers import helper as imported_helper


def helper(value: int) -> int:
    return value + 1


def caller() -> int:
    return helper(1)


class TestMath:
    def test_add(self):
        return caller()
