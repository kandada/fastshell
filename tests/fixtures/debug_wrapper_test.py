import sys, os, subprocess
r = subprocess.run("ls", shell=True, capture_output=True, text=True)
print("LS_OUTPUT:", repr(r.stdout.strip()))
