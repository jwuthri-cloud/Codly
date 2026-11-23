from drv import Session
s = Session("smoke", hard_timeout=60)
s.user("Reply with the single word: pong")
print("result:", s.wait_results(1, 45))
print("exit:", s.close())
