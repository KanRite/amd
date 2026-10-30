# The 4.x address of the Windows installer (ticket 339). 4.x builds name
# `.../v4/install.ps1` in their update bar, but install.ps1 only arrived in
# 5.x, so the address was a 404. It hands over to the current installer:
#
#   irm https://raw.githubusercontent.com/kanrite/rite/v4/install.ps1 | iex
irm https://raw.githubusercontent.com/kanrite/rite/v5/install.ps1 | iex
