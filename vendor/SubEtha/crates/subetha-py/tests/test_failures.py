"""A SubEtha failure raises the exception Python's own calls raise for the
same failure, carrying the operating system's code and the path the call
opened."""

import errno
import io
import os
import sys

import pytest

import subetha


@pytest.fixture
def scratch(tmp_path):
    return lambda name: os.fspath(tmp_path / name)


def test_a_missing_file_raises_file_not_found_with_its_code_and_path(scratch):
    path = scratch("absent")
    with pytest.raises(FileNotFoundError) as caught:
        subetha.Atomic.open(path)
    failure = caught.value
    assert failure.errno == errno.ENOENT
    assert failure.filename == path
    assert failure.strerror.startswith("attaching to the atomic: "), failure.strerror
    assert "os error" not in failure.strerror, "Python prints the code itself"
    if sys.platform == "win32":
        assert failure.winerror is not None, "Windows reports its own code beside errno"


def test_a_file_of_another_shape_raises_layout_mismatch(scratch):
    path = scratch("map")
    made = subetha.HashMap(path, capacity=16, key_size=8, value_size=8)
    with pytest.raises(subetha.LayoutMismatch) as caught:
        subetha.HashMap.open(path, capacity=32, key_size=8, value_size=8)
    failure = caught.value
    assert isinstance(failure, OSError), "except OSError catches it with the other open failures"
    assert failure.errno is None, "the operating system reported nothing"
    assert str(failure).startswith("attaching to the map: "), str(failure)
    assert str(failure).endswith(f": '{path}'"), str(failure)
    del made


def test_a_layout_mismatch_names_the_field_and_both_values(scratch):
    path = scratch("map")
    made = subetha.HashMap(path, capacity=16, key_size=8, value_size=8)
    with pytest.raises(subetha.LayoutMismatch) as caught:
        subetha.HashMap.open(path, capacity=8, key_size=8, value_size=8)
    assert "the file's capacity is 16 where 8 was asked for" in str(caught.value), str(caught.value)
    del made


def test_a_pubsub_of_another_capacity_raises_layout_mismatch(scratch):
    path = scratch("pubsub")
    made = subetha.PubSub(path, capacity=8)
    with pytest.raises(subetha.LayoutMismatch) as caught:
        subetha.PubSub.open(path, capacity=4)
    assert "the file's capacity is 8 where 4 was asked for" in str(caught.value), str(caught.value)
    with pytest.raises(subetha.LayoutMismatch) as caught:
        subetha.PubSub.open(path, capacity=16)
    assert " bytes where " in str(caught.value), str(caught.value)
    assert caught.value.errno is None
    del made


def test_a_wrong_argument_raises_value_error(scratch):
    ring = subetha.SpscRing(scratch("ring"), capacity=4)
    with pytest.raises(ValueError) as caught:
        ring.push(b"x" * (ring.payload_size + 1))
    assert not isinstance(caught.value, OSError)
    assert str(caught.value).startswith("pushing: "), str(caught.value)


def test_a_path_holding_a_nul_raises_value_error(scratch):
    with pytest.raises(ValueError):
        subetha.Atomic(scratch("named") + "\0after", 0)


def test_an_index_past_the_end_raises_index_error(scratch):
    bits = subetha.BitVec(scratch("bits"), capacity_bits=8)
    with pytest.raises(IndexError) as caught:
        bits.get(99)
    assert str(caught.value).startswith("reading a bit: "), str(caught.value)


def test_a_range_past_the_end_of_a_shared_value_raises_index_error(scratch):
    value = subetha.SharedArc(scratch("value"), b"abcd")
    with pytest.raises(IndexError):
        value.read_at(2, 8)
    with pytest.raises(IndexError):
        value.write_at(3, b"xy")
    assert value.read_at(2, 2) == b"cd", "a range inside the value still reads"


def test_an_absent_key_raises_key_error(scratch):
    table = subetha.HashMap(scratch("map"), capacity=16, key_size=8, value_size=8)
    with pytest.raises(KeyError):
        table.compare_exchange(b"k" * 8, b"v" * 8, b"w" * 8)


def test_a_write_to_an_arena_opened_read_only_raises_unsupported_operation(scratch):
    path = scratch("arena")
    writer = subetha.Arena(path, capacity_bytes=4096)
    reader = subetha.Arena.open_read_only(path, capacity_bytes=4096)
    with pytest.raises(io.UnsupportedOperation):
        reader.intern("hello")
    assert writer.intern("hello") is not None, "the writable handle still writes"
