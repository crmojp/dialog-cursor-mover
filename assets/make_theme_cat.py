"""サンプルテーマ「cat」のアセットを生成する。

テーマは exe と同じ階層の assets\\<名前>\\ に置き、次のファイルを持つ。
いずれも任意で、無いものは既定（アプリ内蔵のアイコン、Windows のカーソルと音）
にフォールバックする。

    icon.ico
    cursor_right.ani / cursor_right_fast.ani / cursor_right_slow.ani
    cursor_left.ani  / cursor_left_fast.ani  / cursor_left_slow.ani
    sound.wav

実行すると assets\\theme\\cat\\ に書き出す。
"""

import math
import os
import struct
import wave

import numpy as np
from PIL import Image, ImageDraw

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "theme", "cat")

# ---- 配色 ---------------------------------------------------------------

BODY = (232, 168, 92, 255)      # 茶トラ
BODY_DARK = (196, 132, 62, 255)
STRIPE = (188, 122, 54, 255)
BELLY = (250, 226, 188, 255)
EAR_IN = (244, 176, 176, 255)
NOSE = (232, 132, 140, 255)
EYE = (46, 40, 34, 255)
LIGHT = (255, 255, 255, 255)


# ---- アイコン -----------------------------------------------------------

def draw_icon(img, size):
    d = ImageDraw.Draw(img)
    u = size / 100.0

    def px(*v):
        return [x * u for x in v]

    # 耳
    d.polygon(px(18, 40, 26, 12, 44, 32), fill=BODY, outline=BODY_DARK)
    d.polygon(px(24, 34, 28, 20, 38, 30), fill=EAR_IN)
    d.polygon(px(82, 40, 74, 12, 56, 32), fill=BODY, outline=BODY_DARK)
    d.polygon(px(76, 34, 72, 20, 62, 30), fill=EAR_IN)

    # 顔
    d.ellipse(px(12, 26, 88, 92), fill=BODY, outline=BODY_DARK, width=int(2.5 * u))
    d.ellipse(px(30, 56, 70, 90), fill=BELLY)

    # 額の縞
    for x in (40, 50, 60):
        d.line(px(x, 30, x - 3, 42), fill=STRIPE, width=int(3 * u))

    # 目
    for cx in (36, 64):
        d.ellipse(px(cx - 9, 47, cx + 9, 65), fill=LIGHT, outline=BODY_DARK,
                  width=int(1.4 * u))
        d.ellipse(px(cx - 4, 50, cx + 4, 62), fill=EYE)
        d.ellipse(px(cx - 3, 51, cx - 1, 54), fill=LIGHT)

    # 鼻と口
    d.polygon(px(46, 68, 54, 68, 50, 74), fill=NOSE)
    d.arc(px(40, 70, 50, 80), start=0, end=140, fill=BODY_DARK, width=int(2 * u))
    d.arc(px(50, 70, 60, 80), start=40, end=180, fill=BODY_DARK, width=int(2 * u))

    # ひげ
    w = int(1.8 * u)
    for y0, y1 in ((66, 62), (70, 70), (74, 78)):
        d.line(px(30, y0, 6, y1), fill=BODY_DARK, width=w)
        d.line(px(70, y0, 94, y1), fill=BODY_DARK, width=w)


def make_icon():
    S = 1024
    base = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    draw_icon(base, S)
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    base.resize((256, 256), Image.LANCZOS).save(
        os.path.join(OUT, "icon.ico"), format="ICO", sizes=[(n, n) for n in sizes]
    )
    base.resize((256, 256), Image.LANCZOS).save(os.path.join(OUT, "icon_preview.png"))
    print("wrote icon.ico")


# ---- カーソル -----------------------------------------------------------

SIZE = 32
SS = 8
CW = SIZE * SS
FRAMES = 6
# 速度ごとのコマ送り時間 (jiffies = 1/60 秒)
RATES = {"fast": 3, "normal": 5, "slow": 8}


def draw_cat_frame(idx):
    """右向きに走る猫を 1 フレーム描く。"""
    img = Image.new("RGBA", (CW, CW), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    u = CW / 100.0

    def px(*v):
        return [x * u for x in v]

    phase = idx / FRAMES * 2 * math.pi
    bob = math.sin(phase * 2) * 2.0
    swing_f = math.sin(phase) * 12
    swing_b = math.sin(phase + math.pi) * 12

    # しっぽ（立てて揺らす）
    wag = math.sin(phase + 0.6) * 10
    tail = [(24, 54 + bob), (12, 44 + bob), (6, 26 + bob + wag), (16, 16 + bob + wag)]
    d.line([(x * u, y * u) for x, y in tail], fill=BODY_DARK,
           width=int(5.0 * u), joint="curve")
    d.line([(x * u, y * u) for x, y in tail], fill=BODY,
           width=int(3.0 * u), joint="curve")

    # 脚
    leg_w = int(4.5 * u)
    for base_x, swing in ((34, swing_b), (64, swing_f)):
        for off, sw in ((0, swing), (6, -swing)):
            d.line(px(base_x + off, 62 + bob, base_x + off + sw, 80 + bob),
                   fill=BODY_DARK, width=leg_w)

    # 胴
    d.ellipse(px(22, 38 + bob, 76, 66 + bob), fill=BODY,
              outline=BODY_DARK, width=int(2.2 * u))
    # 縞
    for x in (38, 48, 58):
        d.line(px(x, 40 + bob, x - 3, 56 + bob), fill=STRIPE, width=int(2.6 * u))

    # 耳
    d.polygon(px(64, 34 + bob, 68, 20 + bob, 76, 34 + bob), fill=BODY, outline=BODY_DARK)
    d.polygon(px(78, 34 + bob, 84, 22 + bob, 88, 36 + bob), fill=BODY, outline=BODY_DARK)

    # 頭
    d.ellipse(px(64, 32 + bob, 94, 60 + bob), fill=BODY,
              outline=BODY_DARK, width=int(2.2 * u))
    d.ellipse(px(88, 44 + bob, 96, 52 + bob), fill=NOSE)
    d.ellipse(px(76, 40 + bob, 85, 49 + bob), fill=LIGHT)
    d.ellipse(px(79, 43 + bob, 83, 47 + bob), fill=EYE)

    return img.resize((SIZE, SIZE), Image.LANCZOS)


def make_cur(img, hotspot):
    """RGBA 画像 1 枚から .cur のバイト列を作る。"""
    w, h = img.size
    p = img.load()
    xor = bytearray()
    for y in range(h - 1, -1, -1):
        for x in range(w):
            r, g, b, a = p[x, y]
            xor += bytes((b, g, r, a))
    row_bytes = ((w + 31) // 32) * 4
    and_mask = bytes(row_bytes * h)
    bih = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0,
                      len(xor) + len(and_mask), 0, 0, 0, 0)
    image = bih + bytes(xor) + and_mask
    header = struct.pack("<HHH", 0, 2, 1)
    entry = struct.pack("<BBBBHHII", w, h, 0, 0, hotspot[0], hotspot[1],
                        len(image), 6 + 16)
    return header + entry + image


def riff_chunk(tag, payload):
    data = tag + struct.pack("<I", len(payload)) + payload
    if len(payload) % 2:
        data += b"\x00"
    return data


def make_ani(cur_frames, name, disp_rate):
    anih = struct.pack("<IIIIIIIII", 36, len(cur_frames), len(cur_frames),
                       0, 0, 0, 0, disp_rate, 1)
    info = riff_chunk(b"INAM", name.encode("ascii") + b"\x00")
    info += riff_chunk(b"IART", b"DialogCursorMover\x00")
    list_info = b"LIST" + struct.pack("<I", len(b"INFO") + len(info)) + b"INFO" + info
    frames = b"".join(riff_chunk(b"icon", c) for c in cur_frames)
    list_fram = b"LIST" + struct.pack("<I", len(b"fram") + len(frames)) + b"fram" + frames
    body = b"ACON" + list_info + riff_chunk(b"anih", anih) + list_fram
    return b"RIFF" + struct.pack("<I", len(body)) + body


def verify_ani(path):
    data = open(path, "rb").read()
    assert data[:4] == b"RIFF" and data[8:12] == b"ACON"
    assert struct.unpack("<I", data[4:8])[0] == len(data) - 8
    print(f"  {os.path.basename(path)} ({len(data)} bytes)")


def make_cursors():
    right = [draw_cat_frame(i) for i in range(FRAMES)]
    left = [im.transpose(Image.FLIP_LEFT_RIGHT) for im in right]
    for frames, hs, side in ((right, (SIZE - 4, SIZE // 2), "right"),
                             (left, (3, SIZE // 2), "left")):
        curs = [make_cur(im, hs) for im in frames]
        for speed, rate in RATES.items():
            suffix = "" if speed == "normal" else f"_{speed}"
            path = os.path.join(OUT, f"cursor_{side}{suffix}.ani")
            open(path, "wb").write(make_ani(curs, f"Cat {side} {speed}", rate))
            verify_ani(path)

    strip = Image.new("RGBA", (SIZE * FRAMES, SIZE), (255, 255, 255, 255))
    for i, im in enumerate(right):
        strip.paste(im, (i * SIZE, 0), im)
    strip.resize((SIZE * FRAMES * 3, SIZE * 3), Image.NEAREST).save(
        os.path.join(OUT, "cursor_preview.png"))


# ---- 鳴き声 -------------------------------------------------------------

SR = 44100


def meow(dur=0.42):
    """猫の鳴き声に近い音を合成する。

    実際の猫の声は基本周波数 600-900Hz 程度で、口の開閉により
    共鳴（フォルマント）が上下する。基音を緩やかに上下させ、
    倍音を重ねてからフォルマントの包絡を掛ける。
    """
    n = int(SR * dur)
    t = np.arange(n) / SR
    p = t / dur

    # 基音: 立ち上がりで上がり、後半でゆるやかに下がる
    f0 = 620 + 260 * np.sin(np.pi * np.clip(p / 0.55, 0, 1)) - 120 * np.clip(
        (p - 0.55) / 0.45, 0, 1)
    f0 *= 1.0 + 0.02 * np.sin(2 * np.pi * 5.5 * t)  # ゆるいビブラート

    phase = 2 * np.pi * np.cumsum(f0) / SR
    sig = np.zeros(n)
    # 倍音を重ねる。高次ほど弱く
    for k, amp in enumerate([1.0, 0.55, 0.32, 0.18, 0.10], start=1):
        sig += amp * np.sin(k * phase)

    # フォルマント: 「ミャ」から「アォ」へ変わるように、
    # 中心周波数が下がる帯域強調を掛ける
    center = 1500 - 700 * p
    width = 700.0
    freqs = np.fft.rfftfreq(n, 1 / SR)
    spec = np.fft.rfft(sig)
    shape = np.exp(-((freqs - center.mean()) ** 2) / (2 * width ** 2)) * 1.6 + 0.5
    sig = np.fft.irfft(spec * shape, n=n)

    # 包絡
    env = np.ones(n)
    a = int(SR * 0.030)
    env[:a] *= np.linspace(0, 1, a) ** 0.6
    r = int(SR * 0.16)
    env[-r:] *= np.linspace(1, 0, r) ** 1.4
    sig *= env

    m = np.max(np.abs(sig))
    if m > 0:
        sig = sig / m * 0.5
    return sig


def make_sound():
    sig = meow()
    pcm = (np.clip(sig, -1, 1) * 32767).astype("<i2")
    path = os.path.join(OUT, "sound.wav")
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())
    print(f"wrote sound.wav ({len(pcm) / SR * 1000:.0f}ms)")


def main():
    os.makedirs(OUT, exist_ok=True)
    make_icon()
    make_cursors()
    make_sound()
    print(f"\nテーマを書き出しました: {OUT}")


if __name__ == "__main__":
    main()
