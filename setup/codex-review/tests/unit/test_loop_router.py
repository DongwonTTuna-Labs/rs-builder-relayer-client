from codex_review.loop.router import route_after_stage00, route_after_stage02, route_after_stage04, route_after_stage07


def test_routes_across_key_stages():
    assert route_after_stage00({"route":"run_review"})["run_review"] is True
    assert route_after_stage02({"decisions":[{"action":"needs_human"}]})["route"] == "stop_needs_human"
    assert route_after_stage04({"status":"approved_for_fix"})["route"] == "run_stage05"
    assert route_after_stage07({"pushed":True})["route"] == "record_reentry"
