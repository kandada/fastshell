import subprocess
r = subprocess.run("ls", shell=True, capture_output=True, text=True)
print(r.stdout.strip())
