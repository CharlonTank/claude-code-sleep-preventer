Sudoers NOPASSWD for xtask

Use this to allow `cargo xtask complete-test --skip-notarize --keep-model` to run
without prompting for your password (pmset + cleanup steps only).

1) Optional: remove any broken file first

```bash
sudo rm -f /etc/sudoers.d/claude-xtask
```

2) Run this ONE LINE exactly (do not wrap or add line breaks)

```bash
u=$(id -un); sudo sh -c 'printf "%s ALL=(root) NOPASSWD: /usr/bin/pmset -a disablesleep 0, /usr/bin/pmset -a disablesleep 1, /usr/bin/pmset -a sleep 5, /usr/bin/pmset sleepnow, /bin/rm -f /etc/sudoers.d/claude-pmset\n" "$1" > /etc/sudoers.d/claude-xtask && chmod 440 /etc/sudoers.d/claude-xtask && /usr/sbin/visudo -c -f /etc/sudoers.d/claude-xtask' _ "$u"
```

3) Verify (should say "parsed OK")

```bash
sudo /usr/sbin/visudo -c -f /etc/sudoers.d/claude-xtask
```

4) Undo (if you want to remove the exception)

```bash
sudo rm -f /etc/sudoers.d/claude-xtask
```
