; Vectorize — Inno Setup Installer Script
; Download Inno Setup from: https://jrsoftware.org/isdl.php
; Then: right-click this file → Compile with Inno Setup

[Setup]
AppName=Vectorize
AppVersion=0.1.0
AppPublisher=Vectorize
AppPublisherURL=https://github.com/user/vectorize
DefaultDirName={autopf}\Vectorize
DefaultGroupName=Vectorize
OutputDir=dist
OutputBaseFilename=Vectorize-Setup
Compression=lzma2/ultra64
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayName=Vectorize
; Uncomment when you have a .ico file:
; SetupIconFile=crates\vectorize-gui\assets\vectorize.ico

[Files]
Source: "dist\Vectorize\Vectorize.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "dist\Vectorize\ui_config.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "dist\Vectorize\assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Vectorize"; Filename: "{app}\Vectorize.exe"
Name: "{autodesktop}\Vectorize"; Filename: "{app}\Vectorize.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Run]
Filename: "{app}\Vectorize.exe"; Description: "Launch Vectorize"; Flags: nowait postinstall skipifsilent
