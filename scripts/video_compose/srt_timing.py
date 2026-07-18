"""SRT字幕ファイルから [PARAGRAPH] マーカーの時刻を抽出し、
スライドショーの各表示セグメント (開始秒, 終了秒) を計算する。"""
import re

_PARAGRAPH_BLOCK_RE = re.compile(
    r"\d+\r?\n"
    r"(\d{2}):(\d{2}):(\d{2}),(\d{3})"
    r" --> "
    r"\d{2}:\d{2}:\d{2},\d{3}\r?\n"
    r"\[PARAGRAPH\]"
)


def parse_paragraph_markers(srt_text: str) -> list[float]:
    """SRTテキストから [PARAGRAPH] エントリの開始時刻 (秒) を出現順に返す。"""
    markers = []
    for match in _PARAGRAPH_BLOCK_RE.finditer(srt_text):
        hours, minutes, seconds, millis = (int(g) for g in match.groups())
        markers.append(hours * 3600 + minutes * 60 + seconds + millis / 1000.0)
    return markers


def compute_segments(marker_times: list[float], total_duration_s: float) -> list[tuple[float, float]]:
    """マーカー時刻のリストと音声総時間から、各表示セグメントの (開始秒, 終了秒) を計算する。

    セグメント数 = マーカー数 + 1。
    セグメント1      : 音声開始 (0.0)        〜 1個目のマーカー
    セグメントk (中間): (k-1)個目のマーカー  〜 k個目のマーカー
    最終セグメント   : 最後のマーカー        〜 音声終端 (total_duration_s)

    壊れた/手編集されたSRTを渡すと、検証なしでは負の長さのセグメントが
    ffmpegへそのまま渡ってしまう(review.txt指摘)。マーカーは
    「0より大きい・単調増加・総時間以下」を満たさなければ ValueError にする。
    """
    prev = 0.0
    for i, t in enumerate(marker_times, start=1):
        if t < 0:
            raise ValueError(f"{i}番目の[PARAGRAPH]マーカー時刻が負です: {t}")
        if t <= prev:
            raise ValueError(
                f"{i}番目の[PARAGRAPH]マーカー時刻が単調増加していません: "
                f"{t} は直前の境界 {prev} 以下です"
            )
        if t > total_duration_s:
            raise ValueError(
                f"{i}番目の[PARAGRAPH]マーカー時刻が音声総時間を超えています: "
                f"{t} > {total_duration_s}"
            )
        prev = t

    boundaries = [0.0, *marker_times, total_duration_s]
    return [(boundaries[i], boundaries[i + 1]) for i in range(len(boundaries) - 1)]
