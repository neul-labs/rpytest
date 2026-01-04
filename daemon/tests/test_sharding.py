"""Tests for the sharding module."""

import pytest

from rpytest_daemon.sharding import (
    ShardConfig,
    ShardInfo,
    TestSharder,
    RemoteExecutionConfig,
    LocalExecutor,
)


class TestShardConfig:
    """Tests for ShardConfig dataclass."""

    def test_basic_creation(self):
        config = ShardConfig(shard_index=0, total_shards=4)
        assert config.shard_index == 0
        assert config.total_shards == 4
        assert config.strategy == "hash"  # Default

    def test_with_strategy(self):
        config = ShardConfig(
            shard_index=1,
            total_shards=4,
            strategy="duration_balanced",
        )
        assert config.strategy == "duration_balanced"

    def test_validate_valid(self):
        config = ShardConfig(shard_index=0, total_shards=4)
        assert config.validate() is True

    def test_validate_negative_shard_index(self):
        config = ShardConfig(shard_index=-1, total_shards=4)
        assert config.validate() is False

    def test_validate_zero_total_shards(self):
        config = ShardConfig(shard_index=0, total_shards=0)
        assert config.validate() is False

    def test_validate_shard_index_exceeds_total(self):
        config = ShardConfig(shard_index=4, total_shards=4)
        assert config.validate() is False

    def test_validate_invalid_strategy(self):
        config = ShardConfig(shard_index=0, total_shards=4, strategy="invalid")
        assert config.validate() is False

    def test_to_dict(self):
        config = ShardConfig(shard_index=1, total_shards=4, strategy="round_robin")
        result = config.to_dict()
        assert result == {
            "shard_index": 1,
            "total_shards": 4,
            "strategy": "round_robin",
        }

    def test_from_dict(self):
        data = {
            "shard_index": 2,
            "total_shards": 8,
            "strategy": "duration_balanced",
        }
        config = ShardConfig.from_dict(data)
        assert config.shard_index == 2
        assert config.total_shards == 8
        assert config.strategy == "duration_balanced"

    def test_from_dict_default_strategy(self):
        data = {"shard_index": 0, "total_shards": 4}
        config = ShardConfig.from_dict(data)
        assert config.strategy == "hash"


class TestShardInfo:
    """Tests for ShardInfo dataclass."""

    def test_basic_creation(self):
        info = ShardInfo(node_id="test.py::test_1", shard_index=0)
        assert info.node_id == "test.py::test_1"
        assert info.shard_index == 0
        assert info.estimated_duration_ms == 0

    def test_with_duration(self):
        info = ShardInfo(
            node_id="test.py::test_1",
            shard_index=2,
            estimated_duration_ms=500,
        )
        assert info.estimated_duration_ms == 500


class TestTestSharder:
    """Tests for TestSharder class."""

    def test_init_no_estimates(self):
        sharder = TestSharder()
        assert sharder.duration_estimates == {}

    def test_init_with_estimates(self):
        estimates = {"test.py::test_1": 100, "test.py::test_2": 200}
        sharder = TestSharder(duration_estimates=estimates)
        assert sharder.duration_estimates == estimates

    def test_shard_by_hash_consistency(self):
        """Same test always goes to the same shard."""
        sharder = TestSharder()
        node_ids = ["test.py::test_1", "test.py::test_2", "test.py::test_3"]

        shards1 = sharder.shard_by_hash(node_ids, total_shards=4)
        shards2 = sharder.shard_by_hash(node_ids, total_shards=4)

        assert shards1 == shards2

    def test_shard_by_hash_all_tests_assigned(self):
        sharder = TestSharder()
        node_ids = [f"test.py::test_{i}" for i in range(100)]

        shards = sharder.shard_by_hash(node_ids, total_shards=4)

        # All shards should have keys
        assert len(shards) == 4

        # All tests should be assigned
        all_assigned = []
        for shard_tests in shards.values():
            all_assigned.extend(shard_tests)
        assert sorted(all_assigned) == sorted(node_ids)

    def test_shard_round_robin_even_distribution(self):
        sharder = TestSharder()
        node_ids = [f"test.py::test_{i}" for i in range(8)]

        shards = sharder.shard_round_robin(node_ids, total_shards=4)

        # Each shard should have exactly 2 tests
        for shard_idx, tests in shards.items():
            assert len(tests) == 2

    def test_shard_round_robin_order(self):
        sharder = TestSharder()
        node_ids = ["test_0", "test_1", "test_2", "test_3"]

        shards = sharder.shard_round_robin(node_ids, total_shards=2)

        assert shards[0] == ["test_0", "test_2"]
        assert shards[1] == ["test_1", "test_3"]

    def test_shard_duration_balanced_fallback(self):
        """Falls back to round robin when no duration data."""
        sharder = TestSharder()
        node_ids = ["test_0", "test_1", "test_2", "test_3"]

        result = sharder.shard_duration_balanced(node_ids, total_shards=2)
        expected = sharder.shard_round_robin(node_ids, total_shards=2)

        assert result == expected

    def test_shard_duration_balanced_with_estimates(self):
        sharder = TestSharder(duration_estimates={
            "test_short": 100,
            "test_long": 900,
            "test_medium_1": 300,
            "test_medium_2": 300,
        })

        node_ids = ["test_short", "test_long", "test_medium_1", "test_medium_2"]
        shards = sharder.shard_duration_balanced(node_ids, total_shards=2)

        # Calculate shard durations
        durations = {}
        for idx, tests in shards.items():
            durations[idx] = sum(sharder.duration_estimates.get(t, 100) for t in tests)

        # Shards should be relatively balanced
        # Long (900) + Short (100) = 1000
        # Medium_1 (300) + Medium_2 (300) = 600
        # Or some similar balanced split
        assert max(durations.values()) - min(durations.values()) <= 500

    def test_get_shard_invalid_config(self):
        sharder = TestSharder()
        config = ShardConfig(shard_index=-1, total_shards=4)

        with pytest.raises(ValueError, match="Invalid shard config"):
            sharder.get_shard(["test_1", "test_2"], config)

    def test_get_shard_hash_strategy(self):
        sharder = TestSharder()
        config = ShardConfig(shard_index=0, total_shards=4, strategy="hash")
        node_ids = ["test_1", "test_2", "test_3", "test_4", "test_5"]

        result = sharder.get_shard(node_ids, config)

        # Result should be a subset of node_ids
        assert all(t in node_ids for t in result)

    def test_get_shard_round_robin_strategy(self):
        sharder = TestSharder()
        config = ShardConfig(shard_index=0, total_shards=2, strategy="round_robin")
        node_ids = ["test_0", "test_1", "test_2", "test_3"]

        result = sharder.get_shard(node_ids, config)

        assert result == ["test_0", "test_2"]

    def test_get_shard_unknown_strategy(self):
        sharder = TestSharder()
        config = ShardConfig(shard_index=0, total_shards=4, strategy="unknown")

        # The validate() method catches invalid strategy, so get_shard raises
        # "Invalid shard config" rather than "Unknown sharding strategy"
        with pytest.raises(ValueError, match="Invalid shard config"):
            sharder.get_shard(["test_1"], config)

    def test_get_all_shards(self):
        sharder = TestSharder()
        node_ids = ["test_0", "test_1", "test_2", "test_3"]

        shards = sharder.get_all_shards(node_ids, total_shards=2, strategy="round_robin")

        assert len(shards) == 2
        assert shards[0] == ["test_0", "test_2"]
        assert shards[1] == ["test_1", "test_3"]

    def test_get_shard_info(self):
        sharder = TestSharder(duration_estimates={"test_1": 100, "test_2": 200})
        node_ids = ["test_1", "test_2"]

        info = sharder.get_shard_info(node_ids, total_shards=2, strategy="round_robin")

        assert len(info) == 2
        assert all(isinstance(i, ShardInfo) for i in info)

    def test_estimate_shard_duration(self):
        sharder = TestSharder(duration_estimates={
            "test_1": 100,
            "test_2": 200,
            "test_3": 300,
        })

        duration = sharder.estimate_shard_duration(["test_1", "test_2", "test_3"])
        assert duration == 600

    def test_estimate_shard_duration_with_unknown(self):
        sharder = TestSharder(duration_estimates={"test_1": 100})

        # Unknown test defaults to 100ms
        duration = sharder.estimate_shard_duration(["test_1", "test_unknown"])
        assert duration == 200

    def test_get_balance_report(self):
        sharder = TestSharder(duration_estimates={
            "test_0": 100,
            "test_1": 100,
            "test_2": 100,
            "test_3": 100,
        })
        node_ids = ["test_0", "test_1", "test_2", "test_3"]

        report = sharder.get_balance_report(node_ids, total_shards=2, strategy="round_robin")

        assert report["strategy"] == "round_robin"
        assert report["total_shards"] == 2
        assert report["total_tests"] == 4
        assert len(report["shard_test_counts"]) == 2
        assert len(report["shard_durations_ms"]) == 2
        assert report["count_imbalance_percent"] == 0.0

    def test_get_balance_report_imbalanced(self):
        sharder = TestSharder(duration_estimates={
            "test_long": 900,
            "test_short_1": 100,
            "test_short_2": 100,
        })
        node_ids = ["test_long", "test_short_1", "test_short_2"]

        # Round robin will put test_long in shard 0, short tests in shards 1 and 0
        report = sharder.get_balance_report(node_ids, total_shards=2, strategy="round_robin")

        # Should show some duration imbalance
        assert report["duration_imbalance_percent"] > 0


class TestRemoteExecutionConfig:
    """Tests for RemoteExecutionConfig dataclass."""

    def test_defaults(self):
        config = RemoteExecutionConfig()
        assert config.enabled is False
        assert config.executor_type == "local"
        assert config.executor_config == {}

    def test_to_dict(self):
        config = RemoteExecutionConfig(
            enabled=True,
            executor_type="docker",
            executor_config={"image": "python:3.11"},
        )
        result = config.to_dict()
        assert result == {
            "enabled": True,
            "executor_type": "docker",
            "executor_config": {"image": "python:3.11"},
        }

    def test_from_dict(self):
        data = {
            "enabled": True,
            "executor_type": "ssh",
            "executor_config": {"host": "remote.example.com"},
        }
        config = RemoteExecutionConfig.from_dict(data)
        assert config.enabled is True
        assert config.executor_type == "ssh"
        assert config.executor_config == {"host": "remote.example.com"}


class TestLocalExecutor:
    """Tests for LocalExecutor class."""

    def test_execute(self):
        config = RemoteExecutionConfig(enabled=True, executor_type="local")
        executor = LocalExecutor(config)

        result = executor.execute(
            node_ids=["test.py::test_1"],
            repo_path="/path/to/repo",
            python_path="/usr/bin/python3",
        )

        assert result["executor"] == "local"
        assert result["status"] == "delegated_to_worker_pool"

    def test_get_status(self):
        config = RemoteExecutionConfig(enabled=True, executor_type="local")
        executor = LocalExecutor(config)

        status = executor.get_status()

        assert status["type"] == "local"
        assert status["enabled"] is True
