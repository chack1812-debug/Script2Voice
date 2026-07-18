import pytest

from srt_timing import compute_segments, parse_paragraph_markers


def test_parse_paragraph_markers_extracts_start_times_in_order():
    srt_text = (
        "1\n00:00:00,000 --> 00:00:01,500\nこんにちは\n\n"
        "2\n00:00:01,500 --> 00:00:01,500\n[PARAGRAPH]\n\n"
        "3\n00:00:03,000 --> 00:00:03,800\nさようなら\n\n"
        "4\n00:01:05,250 --> 00:01:05,250\n[PARAGRAPH]\n\n"
    )
    assert parse_paragraph_markers(srt_text) == [1.5, 65.25]


def test_parse_paragraph_markers_returns_empty_list_when_no_markers():
    srt_text = "1\n00:00:00,000 --> 00:00:01,500\nこんにちは\n\n"
    assert parse_paragraph_markers(srt_text) == []


def test_compute_segments_splits_audio_into_marker_count_plus_one_ranges():
    segments = compute_segments([1.5, 65.25], total_duration_s=120.0)
    assert segments == [(0.0, 1.5), (1.5, 65.25), (65.25, 120.0)]


def test_compute_segments_returns_single_segment_when_no_markers():
    segments = compute_segments([], total_duration_s=10.0)
    assert segments == [(0.0, 10.0)]


def test_compute_segments_rejects_non_monotonic_markers():
    # 再現ケース(review.txt指摘): compute_segments([5.0, 3.0], 10.0) は
    # 従来 (5.0, 3.0) という負の長さのセグメントを返していた。
    with pytest.raises(ValueError, match="単調増加"):
        compute_segments([5.0, 3.0], total_duration_s=10.0)


def test_compute_segments_rejects_duplicate_markers():
    with pytest.raises(ValueError, match="単調増加"):
        compute_segments([3.0, 3.0], total_duration_s=10.0)


def test_compute_segments_rejects_negative_marker():
    with pytest.raises(ValueError, match="負"):
        compute_segments([-1.0], total_duration_s=10.0)


def test_compute_segments_rejects_marker_beyond_total_duration():
    with pytest.raises(ValueError, match="総時間"):
        compute_segments([15.0], total_duration_s=10.0)


def test_compute_segments_rejects_marker_at_time_zero():
    # t=0 は暗黙の開始境界と重複し、長さ0の先頭セグメントになるため拒否する。
    with pytest.raises(ValueError, match="単調増加"):
        compute_segments([0.0, 5.0], total_duration_s=10.0)
