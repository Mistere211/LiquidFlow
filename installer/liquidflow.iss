; Inno Setup – baut LiquidFlow-Setup.exe (Zielordner wählbar, Desktop-Icon, Start nach Installation)
[Setup]
AppId={{B7E1F3A2-4C55-4E0A-9D55-1F10A0F10001}
AppName=Liquid Flow
AppVersion=0.3.0
AppPublisher=Mistere211
DefaultDirName={autopf}\Liquid Flow
DefaultGroupName=Liquid Flow
DisableProgramGroupPage=yes
OutputDir=..\dist
OutputBaseFilename=LiquidFlow-Offline-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequiredOverridesAllowed=dialog
UninstallDisplayName=Liquid Flow
UninstallDisplayIcon={app}\liquidflow.exe

[Languages]
Name: "german"; MessagesFile: "compiler:Languages\German.isl"

[Tasks]
Name: "desktopicon"; Description: "Desktop-Symbol erstellen"; GroupDescription: "Zusätzliche Symbole:"

[Files]
Source: "..\target\release\liquidflow.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\pt\*"; DestDir: "{app}\pt"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\Liquid Flow"; Filename: "{app}\liquidflow.exe"
Name: "{autodesktop}\Liquid Flow"; Filename: "{app}\liquidflow.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Clients\StartMenuInternet\LiquidFlow"; ValueType: string; ValueName: ""; ValueData: "Liquid Flow"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Clients\StartMenuInternet\LiquidFlow\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\liquidflow.exe,0"
Root: HKA; Subkey: "Software\Clients\StartMenuInternet\LiquidFlow\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\liquidflow.exe"""
Root: HKA; Subkey: "Software\Clients\StartMenuInternet\LiquidFlow\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "Liquid Flow"
Root: HKA; Subkey: "Software\Clients\StartMenuInternet\LiquidFlow\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Schneller, privater Browser in Rust"
Root: HKA; Subkey: "Software\Clients\StartMenuInternet\LiquidFlow\Capabilities\URLAssociations"; ValueType: string; ValueName: "http"; ValueData: "LiquidFlowURL"
Root: HKA; Subkey: "Software\Clients\StartMenuInternet\LiquidFlow\Capabilities\URLAssociations"; ValueType: string; ValueName: "https"; ValueData: "LiquidFlowURL"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "LiquidFlow"; ValueData: "Software\Clients\StartMenuInternet\LiquidFlow\Capabilities"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\LiquidFlowURL"; ValueType: string; ValueName: ""; ValueData: "Liquid Flow URL"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\LiquidFlowURL"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\LiquidFlowURL\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\liquidflow.exe"" ""%1"""

[Run]
Filename: "ms-settings:defaultapps"; Description: "Standard-Apps öffnen (Liquid Flow als Browser wählen)"; Flags: shellexec postinstall skipifsilent unchecked
Filename: "{app}\liquidflow.exe"; Description: "Liquid Flow starten"; Flags: nowait postinstall skipifsilent
