"""Unit tests for gh_api helpers."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path
from unittest.mock import patch

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / ".github" / "scripts"))

import gh_api  # noqa: E402


class GraphQLPaginatedTest(unittest.TestCase):
    def _stub_pages(self, pages: list[dict]) -> object:
        calls: list[dict] = []

        def fake_gh_graphql(query: str, variables: dict[str, object]) -> dict:
            calls.append(dict(variables))
            return pages[len(calls) - 1]

        return calls, fake_gh_graphql

    def test_single_page(self) -> None:
        pages = [
            {
                "data": {
                    "x": {
                        "nodes": [{"id": 1}, {"id": 2}],
                        "pageInfo": {"hasNextPage": False, "endCursor": None},
                    }
                }
            }
        ]
        calls, fake = self._stub_pages(pages)
        with patch.object(gh_api, "gh_graphql", side_effect=fake):
            nodes = gh_api.gh_graphql_paginated(
                "query", {"foo": "bar"}, extract=lambda p: p["data"]["x"]
            )
        self.assertEqual([n["id"] for n in nodes], [1, 2])
        self.assertEqual(len(calls), 1)
        self.assertIsNone(calls[0]["after"])

    def test_follows_cursor_until_last_page(self) -> None:
        pages = [
            {
                "data": {
                    "x": {
                        "nodes": [{"id": 1}],
                        "pageInfo": {"hasNextPage": True, "endCursor": "C1"},
                    }
                }
            },
            {
                "data": {
                    "x": {
                        "nodes": [{"id": 2}, {"id": 3}],
                        "pageInfo": {"hasNextPage": True, "endCursor": "C2"},
                    }
                }
            },
            {
                "data": {
                    "x": {
                        "nodes": [{"id": 4}],
                        "pageInfo": {"hasNextPage": False, "endCursor": None},
                    }
                }
            },
        ]
        calls, fake = self._stub_pages(pages)
        with patch.object(gh_api, "gh_graphql", side_effect=fake):
            nodes = gh_api.gh_graphql_paginated(
                "query", {"foo": "bar"}, extract=lambda p: p["data"]["x"]
            )
        self.assertEqual([n["id"] for n in nodes], [1, 2, 3, 4])
        self.assertEqual([c["after"] for c in calls], [None, "C1", "C2"])

    def test_missing_cursor_bails_out(self) -> None:
        pages = [
            {
                "data": {
                    "x": {
                        "nodes": [{"id": 1}],
                        "pageInfo": {"hasNextPage": True, "endCursor": None},
                    }
                }
            }
        ]
        _, fake = self._stub_pages(pages)
        with patch.object(gh_api, "gh_graphql", side_effect=fake):
            nodes = gh_api.gh_graphql_paginated(
                "query", {}, extract=lambda p: p["data"]["x"]
            )
        self.assertEqual([n["id"] for n in nodes], [1])

    def test_safety_limit(self) -> None:
        def fake_gh_graphql(query: str, variables: dict[str, object]) -> dict:
            return {
                "data": {
                    "x": {
                        "nodes": [{"id": 1}],
                        "pageInfo": {"hasNextPage": True, "endCursor": "C"},
                    }
                }
            }

        with patch.object(gh_api, "gh_graphql", side_effect=fake_gh_graphql):
            nodes = gh_api.gh_graphql_paginated(
                "query", {}, extract=lambda p: p["data"]["x"]
            )
        self.assertEqual(len(nodes), gh_api.GRAPHQL_PAGE_LIMIT)


if __name__ == "__main__":
    unittest.main()
