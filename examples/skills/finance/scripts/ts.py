# Tushare 取数：apis 查接口 / doc 看文档 / fetch 取数存 parquet。用 `uv run --script` 运行
# /// script
# requires-python = ">=3.10"
# dependencies = ["tushare", "pandas", "pyarrow"]
# ///
"""Tushare 取数入口。用法见 SKILL.md；`uv run` 会按上面的声明自建环境。

  ts.py apis [关键词]                 在接口目录里查接口名
  ts.py doc <接口名>                  打印该接口的入参 / 出参说明
  ts.py fetch <接口名> [k=v ...] [--fields a,b]
                                      取数存 parquet，stdout 只给路径与概要

数据落在 ~/.seekcli/data/tushare/<接口名>/<查询摘要>.parquet：同一查询覆盖
同一个文件，于是整个目录可以直接用 duckdb `read_parquet('<dir>/*.parquet')`
查——这就是「入库」，不另起数据库。
"""

import hashlib
import json
import os
import sys
import time
import urllib.request
from datetime import datetime
from pathlib import Path
from typing import NoReturn

DATA = Path.home() / ".seekcli" / "data" / "tushare"
# 官方 skill 仓库里的接口目录（每行带在线文档链接）。仓库没有声明许可证，
# 所以运行时拉取而不是复制进来。
CATALOG_URL = (
  "https://raw.githubusercontent.com/waditu-tushare/skills/master/tushare/references/"
  "%E6%95%B0%E6%8D%AE%E6%8E%A5%E5%8F%A3.md"
)
CATALOG_TTL = 7 * 24 * 3600
# 限流时的重试：2000 积分档每分钟有调用上限，等一会儿通常就过。
RATE_LIMIT_WAIT = 30
RATE_LIMIT_RETRIES = 2


def die(msg: str) -> NoReturn:
  print(f"ERROR: {msg}", file=sys.stderr)
  sys.exit(1)


def http_get(url: str) -> str:
  req = urllib.request.Request(url, headers={"User-Agent": "seekcli-finance-skill"})
  with urllib.request.urlopen(req, timeout=30) as resp:
    return resp.read().decode("utf-8")


def catalog() -> list[dict[str, str]]:
  path = DATA / "_catalog.md"
  fresh = path.exists() and time.time() - path.stat().st_mtime < CATALOG_TTL
  if not fresh:
    try:
      text = http_get(CATALOG_URL)
      DATA.mkdir(parents=True, exist_ok=True)
      path.write_text(text, encoding="utf-8")
    except Exception as e:
      if not path.exists():
        die(f"无法下载接口目录（{e}）。可直接查 https://tushare.pro/document/2")
      print(f"[ts] 接口目录刷新失败（{e}），使用旧缓存", file=sys.stderr)
  rows = []
  for line in path.read_text(encoding="utf-8").splitlines():
    cells = [c.strip() for c in line.strip().strip("|").split("|")]
    if len(cells) >= 5 and cells[0].startswith("http"):
      rows.append({"doc": cells[0], "api": cells[1], "title": cells[2],
                   "category": cells[3], "desc": cells[4]})
  return rows


def cmd_apis(args: list[str]) -> None:
  keyword = " ".join(args).strip().lower()
  rows = catalog()
  hits = [r for r in rows if not keyword or keyword in " ".join(r.values()).lower()]
  for r in hits[:40]:
    print(f"{r['api']:<22} {r['title']}  [{r['category']}]  {r['desc'][:60]}")
  if len(hits) > 40:
    print(f"... 还有 {len(hits) - 40} 条未显示，换个更具体的关键词")
  if not hits:
    print(f"没有匹配「{keyword}」的接口（目录共 {len(rows)} 条）")


def cmd_doc(args: list[str]) -> None:
  if not args:
    die("用法: ts.py doc <接口名>")
  row = next((r for r in catalog() if r["api"] == args[0]), None)
  if row is None:
    die(f"目录里没有接口 {args[0]}，先用 `ts.py apis <关键词>` 查")
  try:
    print(http_get(row["doc"]))
  except Exception as e:
    die(f"文档下载失败（{e}）：{row['doc']}")


def parse_fetch_args(args: list[str]) -> tuple[str, dict[str, str], str]:
  if not args:
    die("用法: ts.py fetch <接口名> [k=v ...] [--fields a,b]")
  api, params, fields = args[0], {}, ""
  rest = iter(args[1:])
  for a in rest:
    if a == "--fields":
      fields = next(rest, "")
    elif a.startswith("--fields="):
      fields = a.split("=", 1)[1]
    elif "=" in a:
      k, v = a.split("=", 1)
      params[k] = v
    else:
      die(f"参数要写成 k=v：{a}")
  return api, params, fields


def query(api: str, params: dict[str, str], fields: str):
  import tushare as ts

  token = os.environ.get("TUSHARE_TOKEN")
  if not token:
    die("TUSHARE_TOKEN 未设置。先 export TUSHARE_TOKEN=...")
  pro = ts.pro_api(token)
  for attempt in range(RATE_LIMIT_RETRIES + 1):
    try:
      return pro.query(api, fields=fields, **params)
    except Exception as e:
      msg = str(e)
      if "每分钟" in msg and attempt < RATE_LIMIT_RETRIES:
        print(f"[ts] 触发限流，{RATE_LIMIT_WAIT}s 后重试：{msg}", file=sys.stderr)
        time.sleep(RATE_LIMIT_WAIT)
        continue
      # 权限 / 积分不足 / 参数错误都原样上报：模型要据此决定换接口还是问用户。
      die(f"{api} 调用失败：{msg}")
  raise AssertionError("unreachable: the loop either returns or dies")


def cmd_fetch(args: list[str]) -> None:
  api, params, fields = parse_fetch_args(args)
  df = query(api, params, fields)
  key = json.dumps({"params": params, "fields": fields}, sort_keys=True, ensure_ascii=False)
  digest = hashlib.sha256(key.encode()).hexdigest()[:12]
  out = DATA / api / f"{digest}.parquet"
  out.parent.mkdir(parents=True, exist_ok=True)
  fetched_at = datetime.now().astimezone().isoformat(timespec="seconds")
  df.to_parquet(out, index=False)
  meta = {"api": api, "params": params, "fields": fields, "rows": len(df),
          "fetched_at": fetched_at, "source": "tushare.pro"}
  out.with_suffix(".json").write_text(json.dumps(meta, ensure_ascii=False, indent=2), encoding="utf-8")

  print(out)
  print(f"rows={len(df)} fetched_at={fetched_at}")
  if df.empty:
    print("EMPTY: 接口返回 0 行——检查代码 / 日期是否为交易日 / 参数名，不要当成「没有数据」下结论")
    return
  print("columns=" + ",".join(map(str, df.columns)))
  for col in ("trade_date", "end_date", "ann_date", "cal_date"):
    if col in df.columns:
      print(f"{col}: {df[col].min()} ~ {df[col].max()}")


def main() -> None:
  commands = {"apis": cmd_apis, "doc": cmd_doc, "fetch": cmd_fetch}
  if len(sys.argv) < 2 or sys.argv[1] not in commands:
    die(__doc__ or "usage: ts.py apis|doc|fetch ...")
  commands[sys.argv[1]](sys.argv[2:])


if __name__ == "__main__":
  main()
