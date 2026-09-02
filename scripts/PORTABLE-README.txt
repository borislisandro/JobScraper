JobScraper Portable for Windows x64
==================================

Run JobScraper.exe from this folder. Keep JobScraper.exe and the complete
sidecar folder together. The app includes its own Node.js runtime, so a
system Node.js installation is not required.

Requirements
------------

- Windows 10 or Windows 11, x64.
- Microsoft Edge WebView2 Runtime. Current Windows installations normally
  include it. If the app does not open, install the Evergreen WebView2 Runtime
  from Microsoft, then run JobScraper.exe again.

Data and upgrades
-----------------

JobScraper stores its database, settings, and sessions under:

  %LOCALAPPDATA%\JobScraper

Data is not stored beside the executable. Upgrading or moving this portable
folder therefore keeps the same application data. To upgrade, close the app,
extract the new release into a new folder, and run its JobScraper.exe.

Windows startup tasks
---------------------

Start at sign-in and background checks record the current executable path.
Disable both options in JobScraper before moving or deleting this folder. After
the move, launch JobScraper.exe and enable either option again so its Windows
task uses the new path.

Removal
-------

Disable start at sign-in and background checks, close JobScraper, and delete
this folder. Its data under %LOCALAPPDATA%\JobScraper is intentionally retained
unless you remove it separately.
