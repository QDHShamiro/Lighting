import sys

from lighting import defaults as D

if D.WIN:
    from lighting import uia_windows as _impl
elif D.MAC:
    from lighting import uia_macos as _impl
else:
    from lighting import uia_linux as _impl

sys.modules[__name__] = _impl
