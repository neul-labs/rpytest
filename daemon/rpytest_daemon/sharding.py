"""Sharding support for distributed test execution."""

import hashlib
import logging
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Tuple

logger = logging.getLogger(__name__)


@dataclass
class ShardConfig:
    """Configuration for a test shard."""
    shard_index: int
    total_shards: int
    strategy: str = "hash"  # hash, round_robin, duration_balanced

    def validate(self) -> bool:
        """Validate shard configuration."""
        if self.shard_index < 0:
            return False
        if self.total_shards < 1:
            return False
        if self.shard_index >= self.total_shards:
            return False
        if self.strategy not in ("hash", "round_robin", "duration_balanced"):
            return False
        return True

    def to_dict(self) -> Dict:
        """Serialize to dict."""
        return {
            "shard_index": self.shard_index,
            "total_shards": self.total_shards,
            "strategy": self.strategy,
        }

    @classmethod
    def from_dict(cls, data: Dict) -> "ShardConfig":
        """Deserialize from dict."""
        return cls(
            shard_index=data["shard_index"],
            total_shards=data["total_shards"],
            strategy=data.get("strategy", "hash"),
        )


@dataclass
class ShardInfo:
    """Information about a test assignment to shards."""
    node_id: str
    shard_index: int
    estimated_duration_ms: int = 0


class TestSharder:
    """Distributes tests across shards for parallel execution."""

    def __init__(self, duration_estimates: Optional[Dict[str, int]] = None):
        self.duration_estimates = duration_estimates or {}

    def shard_by_hash(
        self,
        node_ids: List[str],
        total_shards: int,
    ) -> Dict[int, List[str]]:
        """Shard tests using consistent hashing.

        Consistent hashing ensures the same test always goes to the same
        shard, which is useful for cache locality.
        """
        shards: Dict[int, List[str]] = {i: [] for i in range(total_shards)}

        for node_id in node_ids:
            # Use MD5 hash for consistent assignment
            hash_val = int(hashlib.md5(node_id.encode()).hexdigest(), 16)
            shard_idx = hash_val % total_shards
            shards[shard_idx].append(node_id)

        return shards

    def shard_round_robin(
        self,
        node_ids: List[str],
        total_shards: int,
    ) -> Dict[int, List[str]]:
        """Shard tests using round-robin distribution.

        Simple even distribution without considering test duration.
        """
        shards: Dict[int, List[str]] = {i: [] for i in range(total_shards)}

        for i, node_id in enumerate(node_ids):
            shard_idx = i % total_shards
            shards[shard_idx].append(node_id)

        return shards

    def shard_duration_balanced(
        self,
        node_ids: List[str],
        total_shards: int,
    ) -> Dict[int, List[str]]:
        """Shard tests balancing total duration across shards.

        Uses duration estimates to distribute tests so each shard
        has approximately equal total runtime.
        """
        if not self.duration_estimates:
            # Fall back to round robin if no duration data
            return self.shard_round_robin(node_ids, total_shards)

        # Get duration for each test
        tests_with_duration = []
        for node_id in node_ids:
            duration = self.duration_estimates.get(node_id, 100)  # Default 100ms
            tests_with_duration.append((node_id, duration))

        # Sort by duration descending (longest first)
        tests_with_duration.sort(key=lambda x: x[1], reverse=True)

        # Use LPT (Longest Processing Time) algorithm
        shard_totals = [0] * total_shards
        shards: Dict[int, List[str]] = {i: [] for i in range(total_shards)}

        for node_id, duration in tests_with_duration:
            # Assign to shard with smallest total duration
            min_shard = min(range(total_shards), key=lambda i: shard_totals[i])
            shards[min_shard].append(node_id)
            shard_totals[min_shard] += duration

        return shards

    def get_shard(
        self,
        node_ids: List[str],
        config: ShardConfig,
    ) -> List[str]:
        """Get tests for a specific shard.

        Args:
            node_ids: All test node IDs.
            config: Shard configuration.

        Returns:
            List of node IDs assigned to this shard.
        """
        if not config.validate():
            raise ValueError(f"Invalid shard config: {config}")

        if config.strategy == "hash":
            shards = self.shard_by_hash(node_ids, config.total_shards)
        elif config.strategy == "round_robin":
            shards = self.shard_round_robin(node_ids, config.total_shards)
        elif config.strategy == "duration_balanced":
            shards = self.shard_duration_balanced(node_ids, config.total_shards)
        else:
            raise ValueError(f"Unknown sharding strategy: {config.strategy}")

        return shards[config.shard_index]

    def get_all_shards(
        self,
        node_ids: List[str],
        total_shards: int,
        strategy: str = "hash",
    ) -> Dict[int, List[str]]:
        """Get test distribution for all shards.

        Args:
            node_ids: All test node IDs.
            total_shards: Number of shards.
            strategy: Sharding strategy.

        Returns:
            Dict mapping shard index to list of node IDs.
        """
        if strategy == "hash":
            return self.shard_by_hash(node_ids, total_shards)
        elif strategy == "round_robin":
            return self.shard_round_robin(node_ids, total_shards)
        elif strategy == "duration_balanced":
            return self.shard_duration_balanced(node_ids, total_shards)
        else:
            raise ValueError(f"Unknown sharding strategy: {strategy}")

    def get_shard_info(
        self,
        node_ids: List[str],
        total_shards: int,
        strategy: str = "hash",
    ) -> List[ShardInfo]:
        """Get detailed shard assignment info for all tests."""
        shards = self.get_all_shards(node_ids, total_shards, strategy)

        info = []
        for shard_idx, shard_nodes in shards.items():
            for node_id in shard_nodes:
                duration = self.duration_estimates.get(node_id, 0)
                info.append(ShardInfo(
                    node_id=node_id,
                    shard_index=shard_idx,
                    estimated_duration_ms=duration,
                ))

        return info

    def estimate_shard_duration(
        self,
        shard_tests: List[str],
    ) -> int:
        """Estimate total duration for a shard."""
        total = 0
        for node_id in shard_tests:
            total += self.duration_estimates.get(node_id, 100)
        return total

    def get_balance_report(
        self,
        node_ids: List[str],
        total_shards: int,
        strategy: str = "hash",
    ) -> Dict:
        """Get a report on how well balanced the sharding is."""
        shards = self.get_all_shards(node_ids, total_shards, strategy)

        shard_counts = []
        shard_durations = []

        for shard_idx, shard_nodes in shards.items():
            shard_counts.append(len(shard_nodes))
            duration = self.estimate_shard_duration(shard_nodes)
            shard_durations.append(duration)

        # Calculate balance metrics
        count_min = min(shard_counts) if shard_counts else 0
        count_max = max(shard_counts) if shard_counts else 0
        duration_min = min(shard_durations) if shard_durations else 0
        duration_max = max(shard_durations) if shard_durations else 0

        count_imbalance = (
            (count_max - count_min) / count_max * 100
            if count_max > 0 else 0
        )
        duration_imbalance = (
            (duration_max - duration_min) / duration_max * 100
            if duration_max > 0 else 0
        )

        return {
            "strategy": strategy,
            "total_shards": total_shards,
            "total_tests": len(node_ids),
            "shard_test_counts": shard_counts,
            "shard_durations_ms": shard_durations,
            "count_imbalance_percent": round(count_imbalance, 2),
            "duration_imbalance_percent": round(duration_imbalance, 2),
            "estimated_wall_time_ms": duration_max,
        }


@dataclass
class RemoteExecutionConfig:
    """Configuration for remote test execution."""
    enabled: bool = False
    executor_type: str = "local"  # local, docker, ssh, kubernetes
    executor_config: Dict = field(default_factory=dict)

    def to_dict(self) -> Dict:
        """Serialize to dict."""
        return {
            "enabled": self.enabled,
            "executor_type": self.executor_type,
            "executor_config": self.executor_config,
        }

    @classmethod
    def from_dict(cls, data: Dict) -> "RemoteExecutionConfig":
        """Deserialize from dict."""
        return cls(
            enabled=data.get("enabled", False),
            executor_type=data.get("executor_type", "local"),
            executor_config=data.get("executor_config", {}),
        )


class RemoteExecutor:
    """Base class for remote test execution.

    Future implementations would include:
    - DockerExecutor: Run tests in containers
    - SSHExecutor: Run tests on remote hosts
    - KubernetesExecutor: Run tests in k8s pods
    """

    def __init__(self, config: RemoteExecutionConfig):
        self.config = config

    def execute(
        self,
        node_ids: List[str],
        repo_path: str,
        python_path: str,
    ) -> Dict:
        """Execute tests remotely.

        Returns dict with execution results.
        """
        raise NotImplementedError("Subclasses must implement execute()")

    def get_status(self) -> Dict:
        """Get executor status."""
        return {
            "type": self.config.executor_type,
            "enabled": self.config.enabled,
        }


class LocalExecutor(RemoteExecutor):
    """Local executor (default, uses worker pool)."""

    def execute(
        self,
        node_ids: List[str],
        repo_path: str,
        python_path: str,
    ) -> Dict:
        """Execute tests locally (delegates to worker pool)."""
        # This is a placeholder - actual execution uses the worker pool
        return {
            "executor": "local",
            "node_ids": node_ids,
            "status": "delegated_to_worker_pool",
        }
