"""アプリアイコン (icon.ico) を生成する。

同心円のターゲットへマウスカーソルが向かう図。
このアプリの機能そのものを表すもので、キャラクター要素は持たせない。

16x16 でも形が読めるよう、リングは 2 本に絞り、線幅を太めに取っている。
1024px で描いてから各サイズへ縮小する。

実行には Pillow が必要（pip install pillow）。
"""

from PIL import Image, ImageDraw

S = 1024
W = S

# 落ち着いた青系。トレイの明暗どちらの背景でも沈まない明度にする
RING = (58, 132, 214, 255)
RING_DARK = (32, 92, 165, 255)
CENTER = (226, 74, 74, 255)
CURSOR = (250, 250, 252, 255)
CURSOR_EDGE = (28, 34, 46, 255)


def draw_icon(img):
    d = ImageDraw.Draw(img)
    u = W / 100.0

    def px(*v):
        return [x * u for x in v]

    cx, cy = 44, 44  # ターゲットの中心

    # --- 外側のリング ---------------------------------------------------
    r1, w1 = 38, 9
    d.ellipse(px(cx - r1, cy - r1, cx + r1, cy + r1),
              outline=RING, width=int(w1 * u))

    # --- 内側のリング ---------------------------------------------------
    r2, w2 = 22, 8
    d.ellipse(px(cx - r2, cy - r2, cx + r2, cy + r2),
              outline=RING_DARK, width=int(w2 * u))

    # --- 中心 -----------------------------------------------------------
    r3 = 8
    d.ellipse(px(cx - r3, cy - r3, cx + r3, cy + r3), fill=CENTER)

    # --- マウスカーソル -------------------------------------------------
    # 中心へ向かう向き（右下から左上）に置く。先端が中心を指す
    tip = (cx + 4, cy + 4)
    arrow = [
        tip,
        (tip[0] + 30, tip[1] + 40),
        (tip[0] + 17, tip[1] + 41),
        (tip[0] + 24, tip[1] + 57),
        (tip[0] + 15, tip[1] + 60),
        (tip[0] + 9, tip[1] + 44),
        (tip[0] - 1, tip[1] + 53),
    ]
    pts = [(x * u, y * u) for x, y in arrow]
    # 縁取りを先に太く描き、内側を塗ることで小サイズでも輪郭が残る
    d.polygon(pts, fill=CURSOR_EDGE)
    d.line(pts + [pts[0]], fill=CURSOR_EDGE, width=int(7 * u), joint="curve")
    inner = [(x * u, y * u) for x, y in arrow]
    d.polygon(inner, fill=CURSOR)
    d.line(inner + [inner[0]], fill=CURSOR_EDGE, width=int(3.2 * u), joint="curve")


def main():
    base = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    draw_icon(base)

    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    base.resize((256, 256), Image.LANCZOS).save(
        "icon.ico", format="ICO", sizes=[(n, n) for n in sizes]
    )
    base.resize((256, 256), Image.LANCZOS).save("icon_preview.png")

    strip = Image.new("RGBA", (16 + 32 + 48 + 30, 48), (255, 255, 255, 255))
    x = 0
    for n in (16, 32, 48):
        small = base.resize((n, n), Image.LANCZOS)
        strip.paste(small, (x, 48 - n), small)
        x += n + 10
    strip.save("icon_sizes.png")
    print("wrote icon.ico / icon_preview.png / icon_sizes.png")


if __name__ == "__main__":
    main()
