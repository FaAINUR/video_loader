#ifndef AppVersion
  #define AppVersion "0.12.0"
#endif
#define ProjectRoot "..\.."

[Setup]
AppId={{AC9DDBDB-57F8-4495-B5ED-1C51ED11F46C}
AppName=VideoLoader
AppVersion={#AppVersion}
AppPublisher=FaAINUR
DefaultDirName={localappdata}\Programs\VideoLoader
DefaultGroupName=VideoLoader
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible and not arm64
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.22000
OutputDir={#ProjectRoot}\dist
OutputBaseFilename=VideoLoader-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\video_loader.exe
CloseApplications=yes
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "Создать ярлык на рабочем столе"; Flags: unchecked

[Files]
Source: "{#ProjectRoot}\target\x86_64-pc-windows-msvc\release\video_loader.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "launch.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "dependencies.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "README-Windows.txt"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{group}\VideoLoader"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\launch.ps1"""; WorkingDir: "{app}"; IconFilename: "{app}\video_loader.exe"
Name: "{group}\VideoLoader - зависимости"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\dependencies.ps1"""; WorkingDir: "{app}"
Name: "{autodesktop}\VideoLoader"; Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\launch.ps1"""; WorkingDir: "{app}"; IconFilename: "{app}\video_loader.exe"; Tasks: desktopicon

[Run]
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\dependencies.ps1"""; Description: "Проверить зависимости и предложить установку через WinGet"; Flags: postinstall skipifsilent unchecked
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\launch.ps1"""; Description: "Запустить VideoLoader"; Flags: postinstall skipifsilent unchecked
