-- Claude Code Sleep Preventer Installer

on run
	set appPath to (path to me as text)
	set posixAppPath to POSIX path of appPath
	set scriptsPath to posixAppPath & "Contents/Resources/scripts/"

	-- Welcome dialog
	set dialogResult to display dialog "Claude Code Sleep Preventer

Keep your Mac awake while Claude Code is working, even with the lid closed.

This will:
- Install hooks to ~/.claude/hooks/
- Set up passwordless sudo for pmset
- Optionally install SwiftBar menu bar HUD

Continue?" buttons {"Cancel", "Install"} default button "Install" with icon note with title "Claude Code Sleep Preventer"

	if button returned of dialogResult is "Cancel" then
		return
	end if

	-- Run install script
	try
		set installScript to scriptsPath & "install-headless.sh"
		do shell script installScript with administrator privileges

		display dialog "Installation complete!

Restart Claude Code to activate.

Menu bar shows:
- Coffee icon = Claude working, sleep disabled
- Zzz icon = Claude idle, sleep enabled" buttons {"OK"} default button "OK" with icon note with title "Success"

	on error errMsg
		display dialog "Installation failed:

" & errMsg buttons {"OK"} default button "OK" with icon stop with title "Error"
	end try
end run
