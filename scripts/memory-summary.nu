#!/usr/bin/env nu

def read-kilobytes [pid: int, key: string] {
  let path = $"/proc/($pid)/status"
  let lines = (try { open $path | lines } catch { [] })
  let matching = ($lines | where {|line| $line | str starts-with $"($key):" })
  if ($matching | is-empty) {
    return null
  }
  let values = ($matching | first | parse -r '^\S+:\s+(?<value>\d+)' | get value)
  if ($values | is-empty) {
    return null
  }
  (($values | first | into int) * 1024)
}

def sample-memory [pid: int] {
  {
    sampled_at: (date now | format date "%+")
    pid: $pid
    rss_bytes: (read-kilobytes $pid "VmRSS")
    anonymous_bytes: (read-kilobytes $pid "RssAnon")
  }
}

def main [
  pid: int
  --samples (-n): int = 10
  --interval-ms: int = 1000
] {
  if $samples < 1 {
    error make { msg: "samples must be greater than zero" }
  }
  if $interval_ms < 0 {
    error make { msg: "interval-ms must not be negative" }
  }

  mut observations = []
  for index in 0..<($samples) {
    let observation = (sample-memory $pid)
    if ($observation.rss_bytes == null or $observation.anonymous_bytes == null) {
      error make { msg: $"process ($pid) exited or does not expose memory metrics" }
    }
    $observations = ($observations | append $observation)
    if $index < ($samples - 1) {
      sleep ($"($interval_ms)ms" | into duration)
    }
  }

  let rss = ($observations | get rss_bytes)
  let anonymous = ($observations | get anonymous_bytes)
  {
    pid: $pid
    sample_count: ($observations | length)
    first_sampled_at: (($observations | first).sampled_at)
    last_sampled_at: (($observations | last).sampled_at)
    rss_start_bytes: ($rss | first)
    rss_peak_bytes: ($rss | math max)
    rss_end_bytes: ($rss | last)
    anonymous_start_bytes: ($anonymous | first)
    anonymous_peak_bytes: ($anonymous | math max)
    anonymous_end_bytes: ($anonymous | last)
  } | to json -r
}
