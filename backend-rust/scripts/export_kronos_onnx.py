#!/usr/bin/env python3
"""
Export Kronos PyTorch model to ONNX format for Rust inference.

Exports three ONNX models:
  1. kronos_tokenizer_encode.onnx — candle sequence → token indices
  2. kronos_decoder.onnx           — token indices → next-token logits
  3. kronos_tokenizer_decode.onnx  — token indices → reconstructed candles

Run inside the Docker container where Kronos + PyTorch are installed:
  python scripts/export_kronos_onnx.py
"""

import os
import sys
import json
import torch
import numpy as np

sys.path.insert(0, "/app/kronos_repo")
from model import KronosTokenizer, Kronos

FINETUNE_DIR = "/app/model_cache/finetuned_nvda"
OUTPUT_DIR = "/app/model_cache/onnx"
LOOKBACK = 60
FEATURES = 6  # OHLCV + amount


class TokenizerEncoder(torch.nn.Module):
    """Wraps tokenizer.encode() for ONNX export."""
    def __init__(self, tokenizer):
        super().__init__()
        self.tokenizer = tokenizer

    def forward(self, x):
        # x: [batch, seq_len, 6]
        z_idx = self.tokenizer.encode(x, half=True)
        s1, s2 = z_idx[0], z_idx[1]
        return s1, s2


class TokenizerDecoder(torch.nn.Module):
    """Wraps tokenizer.decode() for ONNX export."""
    def __init__(self, tokenizer):
        super().__init__()
        self.tokenizer = tokenizer

    def forward(self, s1, s2):
        # s1, s2: [batch, n] int64 token indices
        reconstructed = self.tokenizer.decode(s1, s2)
        return reconstructed


class DecoderStep(torch.nn.Module):
    """Wraps one forward pass of the Kronos decoder for ONNX export."""
    def __init__(self, model):
        super().__init__()
        self.model = model

    def forward(self, s1, s2, stamp):
        # s1, s2: [batch, seq_len] int64
        # stamp: [batch, seq_len, 5]
        s1_logits, s2_logits = self.model(
            s1, s2,
            stamp=stamp,
            use_teacher_forcing=True,
            s1_targets=s1,
        )
        return s1_logits, s2_logits


def main():
    os.makedirs(OUTPUT_DIR, exist_ok=True)
    device = "cuda:0" if torch.cuda.is_available() else "cpu"
    print(f"Device: {device}")

    # Load models
    tok_path = os.path.join(FINETUNE_DIR, "tokenizer")
    pred_path = os.path.join(FINETUNE_DIR, "predictor")

    if os.path.exists(tok_path) and os.path.exists(pred_path):
        tokenizer = KronosTokenizer.from_pretrained(tok_path).to(device)
        model = Kronos.from_pretrained(pred_path).to(device)
        print("Loaded fine-tuned model")
    else:
        tokenizer = KronosTokenizer.from_pretrained("NeoQuasar/Kronos-Tokenizer-base").to(device)
        model = Kronos.from_pretrained("NeoQuasar/Kronos-small").to(device)
        print("Loaded base model")

    tokenizer.eval()
    model.eval()

    batch = 1
    seq_len = LOOKBACK

    # ── Export 1: Tokenizer Encode ─────────────────────────────
    print("\nExporting tokenizer encoder...")
    encoder = TokenizerEncoder(tokenizer).to(device)
    encoder.eval()

    dummy_x = torch.randn(batch, seq_len, FEATURES, device=device)

    try:
        torch.onnx.export(
            encoder,
            (dummy_x,),
            os.path.join(OUTPUT_DIR, "kronos_tokenizer_encode.onnx"),
            input_names=["features"],
            output_names=["s1_tokens", "s2_tokens"],
            dynamic_axes={
                "features": {0: "batch", 1: "seq_len"},
                "s1_tokens": {0: "batch", 1: "token_len"},
                "s2_tokens": {0: "batch", 1: "token_len"},
            },
            opset_version=17,
            do_constant_folding=True,
        )
        print("  -> kronos_tokenizer_encode.onnx exported")
    except Exception as e:
        print(f"  FAILED: {e}")
        print("  Falling back to TorchScript trace...")
        traced = torch.jit.trace(encoder, (dummy_x,))
        traced.save(os.path.join(OUTPUT_DIR, "kronos_tokenizer_encode.pt"))
        print("  -> kronos_tokenizer_encode.pt saved (TorchScript)")

    # ── Export 2: Decoder ──────────────────────────────────────
    print("\nExporting decoder...")
    decoder = DecoderStep(model).to(device)
    decoder.eval()

    # Get token dimensions from a test encode
    with torch.no_grad():
        s1_test, s2_test = encoder(dummy_x)
    token_len = s1_test.shape[1]
    dummy_stamp = torch.randn(batch, token_len, 5, device=device)

    try:
        torch.onnx.export(
            decoder,
            (s1_test, s2_test, dummy_stamp),
            os.path.join(OUTPUT_DIR, "kronos_decoder.onnx"),
            input_names=["s1_tokens", "s2_tokens", "stamps"],
            output_names=["s1_logits", "s2_logits"],
            dynamic_axes={
                "s1_tokens": {0: "batch", 1: "seq_len"},
                "s2_tokens": {0: "batch", 1: "seq_len"},
                "stamps": {0: "batch", 1: "seq_len"},
                "s1_logits": {0: "batch", 1: "seq_len"},
                "s2_logits": {0: "batch", 1: "seq_len"},
            },
            opset_version=17,
            do_constant_folding=True,
        )
        print("  -> kronos_decoder.onnx exported")
    except Exception as e:
        print(f"  FAILED: {e}")
        print("  Falling back to TorchScript trace...")
        traced = torch.jit.trace(decoder, (s1_test, s2_test, dummy_stamp))
        traced.save(os.path.join(OUTPUT_DIR, "kronos_decoder.pt"))
        print("  -> kronos_decoder.pt saved (TorchScript)")

    # ── Export 3: Tokenizer Decode ─────────────────────────────
    print("\nExporting tokenizer decoder...")
    tok_decoder = TokenizerDecoder(tokenizer).to(device)
    tok_decoder.eval()

    try:
        torch.onnx.export(
            tok_decoder,
            (s1_test, s2_test),
            os.path.join(OUTPUT_DIR, "kronos_tokenizer_decode.onnx"),
            input_names=["s1_tokens", "s2_tokens"],
            output_names=["candles"],
            dynamic_axes={
                "s1_tokens": {0: "batch", 1: "token_len"},
                "s2_tokens": {0: "batch", 1: "token_len"},
                "candles": {0: "batch", 1: "seq_len"},
            },
            opset_version=17,
            do_constant_folding=True,
        )
        print("  -> kronos_tokenizer_decode.onnx exported")
    except Exception as e:
        print(f"  FAILED: {e}")
        print("  Falling back to TorchScript trace...")
        traced = torch.jit.trace(tok_decoder, (s1_test, s2_test))
        traced.save(os.path.join(OUTPUT_DIR, "kronos_tokenizer_decode.pt"))
        print("  -> kronos_tokenizer_decode.pt saved (TorchScript)")

    # ── Verify ─────────────────────────────────────────────────
    print("\n=== Export Summary ===")
    for f in os.listdir(OUTPUT_DIR):
        path = os.path.join(OUTPUT_DIR, f)
        size_mb = os.path.getsize(path) / 1024 / 1024
        print(f"  {f}: {size_mb:.1f} MB")

    # Save metadata
    meta = {
        "lookback": LOOKBACK,
        "features": FEATURES,
        "token_len": int(token_len),
        "device": device,
        "vocab_size": getattr(model.head, "vocab_size", "unknown"),
    }
    with open(os.path.join(OUTPUT_DIR, "metadata.json"), "w") as f:
        json.dump(meta, f, indent=2, default=str)

    print("\nDone! Models saved to", OUTPUT_DIR)


if __name__ == "__main__":
    main()
