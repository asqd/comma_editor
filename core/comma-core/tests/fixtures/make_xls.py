"""Writes sales.xls, the Excel 97–2003 workbook the tests read.

Run with xlwt 1.3: `python3 make_xls.py`. The file is committed; run this only to change it.
"""
import datetime

import xlwt

book = xlwt.Workbook(encoding="utf-8")
money = xlwt.easyxf(num_format_str='#,##0.00\\ "₽"')
percent = xlwt.easyxf(num_format_str="0.0%")
date = xlwt.easyxf(num_format_str="DD.MM.YYYY")
time = xlwt.easyxf(num_format_str="HH:MM")

sales = book.add_sheet("Продажи")
for column, name in enumerate(["Товар", "Цена", "Кол-во", "Сумма", "Доля", "Дата", "Время", "Есть"]):
    sales.write(0, column, name)
rows = [("Чай", 250.5, 4), ("Кофе", 890.0, 2), ("Сахар", 75.25, 10)]
for index, (name, price, quantity) in enumerate(rows, start=1):
    sales.write(index, 0, name)
    sales.write(index, 1, price, money)
    sales.write(index, 2, quantity)
    sales.write(index, 3, xlwt.Formula("B%d*C%d" % (index + 1, index + 1)), money)
    sales.write(index, 4, 0.25, percent)
    sales.write(index, 5, datetime.date(2025, 1, index), date)
    sales.write(index, 6, 0.75, time)
    sales.write(index, 7, index % 2 == 1)
sales.write_merge(5, 5, 0, 2, "Итого")
sales.write(5, 3, xlwt.Formula("SUM(D2:D4)"), money)
sales.write(7, 0, "Длинный текст: " + "абвгд" * 3000)
sales.col(0).width = 256 * 20

hidden = book.add_sheet("Служебный")
hidden.write(0, 0, "секрет")
hidden.visibility = 1

notes = book.add_sheet("Заметки")
notes.write(2, 1, "Текст начинается с B3")
notes.write(3, 1, 1234567.891)

book.save("sales.xls")
