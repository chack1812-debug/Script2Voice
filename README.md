# Script2Voice

テキストから音声・動画を生成する、Rust製のText-to-Speech（TTS）パイプラインです。
開発・運営: [宇摩コードワークス（UmaCodeWorks）](https://umacodeworks.jp)

## 紹介動画

Script2Voiceの概要と使い方を紹介した動画です。

[![Script2Voice 紹介動画](https://img.youtube.com/vi/nM6yLUMByTA/maxresdefault.jpg)](https://youtu.be/nM6yLUMByTA)

## 概要

Script2Voiceは、台本形式のテキストを解析し、音声合成エンジン（VOICEVOX / AivisSpeech 等）と連携して音声を生成するパイプラインです。まとまった分量のテキストを継続的に処理する用途を想定し、処理速度とメモリ安定性を重視してRustで実装しています。

挿絵等の静止画素材をあわせて用意することで、生成した音声と画像を組み合わせた動画（スライドショー形式）としての書き出しにも対応しています。

## 主な機能

- 台本形式テキスト（`@scene` / `@pause` / `@asset` / `@cast` / `@script` 等の記法）の解析
- VOICEVOX / AivisSpeech 等の音声合成エンジンとの連携
- 音声の書き出し
- 静止画素材と音声を組み合わせた動画（スライドショー）生成（ffmpeg連携）

## 構成

Cargo workspaceとして、以下のクレートに分割されています。

| クレート | 役割 |
|---|---|
| `s2v-core` | 台本解析・コア処理 |
| `s2v-engines` | 音声合成エンジン連携 |
| `s2v-audio` | 音声処理 |
| `s2v-export` | 書き出し処理 |
| `s2v-video` | 動画生成（画像素材＋音声） |
| `s2v-gui` | GUI |

## 取扱説明書

インストール方法・CLIの使い方・台本の書き方・動画への書き出し（`compose`コマンド）・空間音響パラメータまで網羅した詳細マニュアルを同梱しています。

- [docs/manual.html](./docs/manual.html)

## ライセンス

本ソフトウェアは無償でご利用いただけますが、改変・改変版の再配布は禁止されています。機能のご要望・不具合報告は、コード改変ではなくお問い合わせ窓口までご連絡ください。詳細は [LICENSE.md](./LICENSE.md) をご覧ください。

## 開発・運営

[宇摩コードワークス（UmaCodeWorks）](https://umacodeworks.jp) — 村上孝伸
