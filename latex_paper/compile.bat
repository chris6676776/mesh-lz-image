@echo off
echo Compiling LaTeX Thesis...
pdflatex main.tex
bibtex main
pdflatex main.tex
pdflatex main.tex
echo Done! Check main.pdf!
pause
