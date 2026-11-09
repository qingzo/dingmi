"""把文渊圆体裁成叮咪用的「Dingmi Rounded」（见 docs/方案/UI设计.md U2 字体）。

用法：python scripts/build-font.py <WenYuanRoundedSCVF.ttf>
  字体从 https://github.com/takushun-wu/WenYuanFonts/releases 下 WenYuanRoundedSCVF.ttf（可变字体，TTF 那个）。
  要先 pip install fonttools brotli。换了版本记得改 apps/desktop/ui/fonts/OFL.txt 开头的版本号和 scripts/licenses.mjs。
产物：apps/desktop/ui/fonts/dingmi-rounded.woff2（可变字体，粗细 400–700）。
留哪些字：GB2312 的全部汉字和符号（约 7400 个，覆盖通用规范汉字表一、二级里的常用字）、
英文数字和常用标点，再加上界面代码里用到的每个字。
"""
import pathlib
import re
import sys

from fontTools.subset import Options, Subsetter
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "apps/desktop/ui/fonts/dingmi-rounded.woff2"
NAME = "Dingmi Rounded"


def gb2312() -> set[str]:
    chars = set()
    for hi in range(0xA1, 0xF8):
        for lo in range(0xA1, 0xFF):
            try:
                chars.add(bytes([hi, lo]).decode("gb2312"))
            except UnicodeDecodeError:
                pass
    return chars


def used_in_code() -> set[str]:
    chars = set()
    for d in ("apps/desktop/ui", "apps/mobile", "shared", "design/ui"):
        for p in (ROOT / d).rglob("*"):
            if p.suffix in (".ts", ".html", ".css", ".js") and "node_modules" not in p.parts:
                chars |= set(re.findall(r"[　-鿿＀-￯]", p.read_text(encoding="utf-8", errors="ignore")))
    return chars


def main() -> None:
    font = TTFont(sys.argv[1])
    text = gb2312() | used_in_code() | {chr(c) for c in range(0x20, 0x7F)}
    text |= set("·—–…‘’“”、。，．：；！？（）《》〈〉「」『』【】〔〕×÷±≈≠≤≥°℃‰￥€→←↑↓✓✕")
    opts = Options()
    # tnum：时间、计数用等宽数字（font-variant-numeric: tabular-nums），文渊的数字平时是不等宽的；kern：英文字距
    opts.layout_features = ["liga", "locl", "ccmp", "calt", "rclt", "kern", "mark", "mkmk", "tnum"]
    opts.name_IDs = ["*"]
    opts.notdef_outline = True
    sub = Subsetter(opts)
    sub.populate(text="".join(sorted(text)))
    sub.subset(font)
    # 先裁字再定粗细（字少了快很多）：只留 400–700，不要斜体
    font = instancer.instantiateVariableFont(font, {"wght": (400, 700), "ital": 0})
    version = font["head"].fontRevision

    # OFL 要求裁剪过的字体改名
    names = font["name"]
    for rec in names.names:
        if rec.nameID in (1, 4, 16, 21):
            rec.string = NAME
        elif rec.nameID == 6:
            rec.string = NAME.replace(" ", "")
        elif rec.nameID == 3:
            rec.string = f"{NAME}; subset of WenYuan Rounded SC {version:.3f}"
        elif rec.nameID > 255 and "WenYuan" in rec.toUnicode():
            # 各粗细的 PostScript 名，比如 WenYuanRoundedSCVF-Bold
            rec.string = re.sub(r"WenYuanRoundedSC(VF)?", NAME.replace(" ", ""), rec.toUnicode())
    if "STAT" in font:
        del font["STAT"]

    OUT.parent.mkdir(parents=True, exist_ok=True)
    font.flavor = "woff2"
    font.save(OUT)
    print(f"{OUT.relative_to(ROOT)}: {len(text)} 字，{OUT.stat().st_size / 1024 / 1024:.2f} MB")


if __name__ == "__main__":
    main()
