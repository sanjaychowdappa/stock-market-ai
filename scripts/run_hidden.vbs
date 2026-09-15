' Stock Market AI - windowless launcher for the scheduled tasks.
'
' Task Scheduler runs these actions in the interactive session. It has to:
' auto_start.bat launches Docker Desktop, and Docker Desktop must land in
' the user's own desktop session, not session 0. But an interactive action
' of "cmd.exe /c script.bat" opens a console window for the life of the
' script, and StockMarketAI_Start repeats every 15 minutes so the machine
' catches up after sleep. Result: a black window flashing on screen four
' times an hour, all day (reported 2026-09-15).
'
' wscript.exe is a GUI-subsystem program. It owns no console, and
' WshShell.Run with window style 0 creates the child with its window
' hidden from the start. Nothing appears. powershell -WindowStyle Hidden
' is NOT equivalent: it allocates a console first and hides it a moment
' later, so it still flashes.
'
' Usage from a task action:
'   wscript.exe //B //Nologo "<this file>" <program> [args...]
' Arguments pass through in order; any that contains a space is quoted.
' //B suppresses script-error dialogs so a failure can never hang the
' task waiting on a message box. The child's exit code is returned.

Option Explicit
Dim sh, cmd, arg, i, rc
If WScript.Arguments.Count = 0 Then WScript.Quit 2
cmd = ""
For i = 0 To WScript.Arguments.Count - 1
    arg = WScript.Arguments(i)
    If InStr(arg, " ") > 0 Then arg = """" & arg & """"
    If i > 0 Then cmd = cmd & " "
    cmd = cmd & arg
Next
Set sh = CreateObject("WScript.Shell")
rc = sh.Run(cmd, 0, True)
WScript.Quit rc
