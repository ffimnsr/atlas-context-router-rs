import os.path as osp
from .helpers import helper as imported_helper


@cached
def helper(value: int) -> int:
    return value + 1


def caller() -> int:
    return helper(1)


@dataclass
class TestMath:
    def test_add(self):
        return caller()
