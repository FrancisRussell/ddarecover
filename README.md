# Ddarecover

Ddarecover is a tool that attempts to copy a block device to an output file.
The state of recovery is stored in a mapfile which is compatible with the [GNU
Ddrescue](https://www.gnu.org/software/ddrescue/) tool. Ddarecover uses the
Linux asynchronous IO interface to submit multiple reads in parallel.
Therefore, this tool is *Linux only*. For drives that have large numbers of
read errors distributed in a non-localised fashion, this may provide a
significant throughput increase. In most situations, this is not the case, and
you're probably better off using Ddrescue.

## Example

```
$ cargo build --release
   Compiling ddarecover v0.1.0 (file:///mnt/recovery/ddarecover)
    Finished release [optimized] target(s) in 17.97 secs
$ sudo ./target/release/ddarecover -i /dev/sda -o ./drive.img -m ./drive.map 
Press Ctrl+C to exit.

          Phase: Copying (pass 1)
           ipos: 28532 MiB               rescued: 415354 MiB                  bad: 12124 MiB
      non-tried: 49463 MiB           non-trimmed: 0 B                 non-scraped: 0 B
      read rate: 22460 B/s            error rate: 10788 B/s            total rate: 33248 B/s
       run time: 3m 33s             last success: 0s ago                remaining: 18d 1h
```

## Disclaimer

This code has not been extensively tested. It is also highly unpolished. It may
incorrectly recover data or damage data recovered using other tools. If you use
this tool after a partial recovery with Ddrescue, It is strongly recommended
that you invoke this tool using copies of the drive image and map file. If you
trust [BTRFS](https://btrfs.wiki.kernel.org/index.php/Main_Page), its
copy-on-write behaviour for files may prove useful for creating low-overhead
copies of large drive images.
