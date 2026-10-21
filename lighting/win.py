import sys

from lighting import defaults as D

if D.WIN:
    from lighting import win_windows as _impl
elif D.MAC:
    from lighting import win_macos as _impl
else:
    from lighting import win_linux as _impl

sys.modules[__name__] = _impl
