"""Draw the independent side-elevation and lamp-yard plates in grayscale SVG."""
from pathlib import Path
from random import Random

ROOT = Path(__file__).resolve().parent
R = Random(1937)
W = 1900


def path(d, fill, stroke="none", width=1, opacity=1):
    return f'<path d="{d}" fill="{fill}" stroke="{stroke}" stroke-width="{width}" opacity="{opacity}"/>'


def line(x, y, end_x, end_y, color, width=1, opacity=1):
    return path(f"M{x:.1f} {y:.1f}L{end_x:.1f} {end_y:.1f}", "none", color, width, opacity)


def rect(x, y, w, h, color, opacity=1):
    return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{color}" opacity="{opacity}"/>'


def gradient(name, stops, vertical=True):
    axis = 'x2="0" y2="1"' if vertical else 'x2="1" y2="0"'
    stops = ''.join(f'<stop offset="{p}" stop-color="{color}" stop-opacity="{a}"/>' for p, color, a in stops)
    return f'<linearGradient id="{name}" {axis}>{stops}</linearGradient>'


def radial(name, color, strength):
    return (f'<radialGradient id="{name}"><stop stop-color="{color}" stop-opacity="{strength}"/>'
            f'<stop offset=".4" stop-color="{color}" stop-opacity="{strength/2}"/>'
            f'<stop offset="1" stop-color="{color}" stop-opacity="0"/></radialGradient>')


def ellipse(x, y, w, h, fill, opacity=1):
    return f'<ellipse cx="{x:.1f}" cy="{y:.1f}" rx="{w:.1f}" ry="{h:.1f}" fill="{fill}" opacity="{opacity}"/>'


def save(name, view, art, defs=""):
    x, y, w, h = view
    text = (f'<!-- Original illustrated plate for The Morning Shift. -->\n'
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="{x} {y} {w} {h}">\n'
            f'<defs>{defs}</defs>\n' + '\n'.join(art) + '\n</svg>\n')
    (ROOT / name).write_text(text)


def glass(x, y, w, h, lit=True):
    art = [rect(x-9, y-8, w+18, h+21, "#131313"),
           rect(x, y, w, h, "url(#glass)" if lit else "#393939")]
    for _ in range(85):
        px, py = R.uniform(x, x+w-5), R.uniform(y, y+h-12)
        art.append(line(px, py, px+R.uniform(-2, 2), min(y+h, py+R.uniform(5, 44)),
                        R.choice(("#121212", "#dddddd")), R.uniform(.35, 1.5), .16))
    for col in range(1, 5):
        px = x+w*col/5
        art.extend((line(px, y, px, y+h, "#171717", 5), line(px+4, y, px+4, y+h, "#8b8b8b", .8, .7)))
    for row in range(1, 6):
        py = y+h*row/6
        art.extend((line(x, py, x+w, py, "#191919", 5), line(x, py+4, x+w, py+4, "#a0a0a0", .75, .54)))
    art.append(line(x-9, y+h+7, x+w+9, y+h+7, "#a1a1a1", 2, .6))
    return art


def side_elevation():
    defs = gradient("wall", [(0,"#292929",1),(.44,"#454545",1),(1,"#1d1d1d",1)], False)
    defs += gradient("glass", [(0,"#484848",1),(.4,"#999999",1),(.68,"#777777",1),(1,"#383838",1)])
    defs += gradient("door", [(0,"#171717",1),(.43,"#3b3b3b",1),(1,"#1a1a1a",1)])
    arts = [rect(0, 0, W, 692, "url(#wall)")]
    for y in range(5, 705, 23):
        arts.extend((line(0,y,W,y,"#171717",1,.7),line(0,y+2,W,y+2,"#828282",.7,.42)))
        for x in range(-48+(y//23%2)*39,W,78):
            arts.append(line(x,y,x,y+22,"#212121",1,.6))
    for _ in range(1450):
        x,y=R.uniform(0,W),R.uniform(70,690)
        arts.append(line(x,y,x+R.uniform(-2,2),y+R.uniform(1,28),
                         R.choice(("#171717","#737373","#909090")), R.uniform(.25,2.3), R.uniform(.06,.4)))
    for x in [100, 355, 985, 1260, 1525, 1750]:
        arts.extend(glass(x, 285 if x<900 else 267, 182, 253, x not in (985,1750)))
    arts.extend([path("M560 683V344Q560 279 638 269H817Q899 285 899 344V683Z","#151515","#787878",2),
                 path("M581 686V353Q581 300 654 291H806Q873 311 873 353V686Z","url(#door)","#2c2c2c",3),
                 line(734,291,734,686,"#181818",4), line(739,299,739,685,"#737373",1,.62)])
    for y in range(353,675,17):
        arts.extend((line(583,y,871,y,"#141414",1.5,.71),line(583,y+2,871,y+2,"#727272",.65,.63)))
    arts.extend([line(0,568,W,568,"#151515",18),line(0,561,W,561,"#848484",2,.54),
                 path("M-15 244H498Q521 246 521 274V637M923 160V634", "none","#141414",13),
                 path("M-15 239H501Q525 240 525 271V635M927 160V634", "none","#747474",1.6,.7),
                 rect(610,373,48,29,"#222222"),rect(619,383,29,2,"#777777"),rect(619,389,20,1,"#666666")])
    save("track-wall.svg",(0,0,W,692),arts,defs)


def pavement(name, side):
    ymin=666 if side else 568
    defs=gradient("asphalt",[(0,"#777777",1),(.23,"#3b3b3b",1),(.69,"#222222",1),(1,"#141414",1)])
    defs+=radial("wet","#a5a5a5",.46)+radial("ink","#080808",.63)
    art=[]
    if not side:
        # Keep the paving behind the masonry line, with the lane receding into the right-hand yard.
        edge = "M0 704L637 655 875 618 1025 614 1180 601 1500 618 1900 643V930H0Z"
        defs += '<clipPath id="paving-edge">'+path(edge,"white")+'</clipPath>'
        art.append('<g clip-path="url(#paving-edge)">')
    art.append(rect(0,ymin,W,930-ymin,"url(#asphalt)"))
    if side:
        art.extend([path("M0 685L1900 681 1900 696 0 711Z","#1a1a1a"),line(0,708,1900,693,"#939393",2,.81),
                    ellipse(635,760,470,112,"url(#ink)"),ellipse(1290,734,490,75,"url(#wet)",.72)])
        for y in [734,780,842,910]:
            art.extend((line(0,y,1900,y-24,"#171717",2,.8),line(0,y+4,1900,y-20,"#a0a0a0",.7,.37)))
    else:
        art.extend([path("M0 651L1085 605 1900 691 1900 722 1110 630 0 712Z","#181818"),
                    line(0,710,1110,629,"#888888",2,.64),ellipse(817,740,366,103,"url(#wet)"),
                    ellipse(1280,681,423,56,"url(#wet)",.68)])
        for x in [-310,-130,310,510,850,1020,1430,1680,1930,2350]:
            art.extend((line(1180,605,x,930,"#111111",3,.74),line(1181,607,x+4,930,"#797979",1,.57)))
        for y in [635,652,679,710,754,804,876]:
            art.append(line(0,y+28,1900,y-7,"#121212",1,.62))
    for _ in range(1250):
        x=R.uniform(0,W)
        y=ymin+R.random()**1.4*(930-ymin)
        l=1+(y-ymin)*R.uniform(.008,.23)
        near=abs(x-(1250 if side else 830))
        if R.random() < .32 and near<290:
            color,alpha="#b6b6b6",R.uniform(.13,.5)
        else:
            color,alpha=R.choice(("#080808","#222222","#878787","#acacac")),R.uniform(.07,.26)
        art.append(line(x,y,x+l,y+R.uniform(-1.4,1.4),color,R.uniform(.35,2.1),alpha))
    if not side:
        art.append('</g>')
    save(name,(0,ymin,W,930-ymin),art,defs)


def steelwork():
    defs=gradient("steel",[(0,"#464646",1),(.22,"#202020",1),(.74,"#141414",1),(1,"#0b0b0b",1)])
    defs+=gradient("post",[(0,"#131313",1),(.18,"#555555",1),(.45,"#292929",1),(.84,"#141414",1),(1,"#666666",1)],False)
    art=[path("M-20 36L1512 79 1490 171 1290 239 925 217 897 248 865 236 838 199-20 171Z","url(#steel)"),
         path("M-20 104L1494 129 1488 151-20 130Z","#171717","#525252",1.5),
         path("M826 175H917L891 239 887 717 906 733 901 752 848 757 830 744 848 724 850 240Z",
              "url(#post)","#111111",2),
         path("M848 241L850 726 839 740 900 734M882 717L886 234 903 200M-20 164L833 188M920 207L1288 235 1493 166",
              "none","#969696",1.4,.58)]
    for x in range(0,1500,58):
        y=113+x*.017
        art.extend((line(x,y-22,x+37,y+35,"#575757",1.5,.58),ellipse(x+8,y,2,2,"#a5a5a5",.48)))
    for y in [257,418,598,703,729]:
        art.extend((line(849,y,887,y-2,"#090909",4),line(850,y+2,883,y,"#666666",.9,.65)))
    save("track-steel.svg",(-20,0,1544,771),art,defs)


def lamp_yard():
    defs=gradient("sky",[(0,"#282828",1),(.41,"#565656",1),(1,"#a4a4a4",1)])
    defs+=gradient("facade",[(0,"#2f2f2f",1),(.7,"#484848",1),(1,"#181818",1)])
    defs+=gradient("glass",[(0,"#626262",1),(.46,"#aaaaaa",1),(1,"#4a4a4a",1)])
    defs+=radial("haze","#d3d3d3",.51)
    art=[rect(0,0,W,682,"url(#sky)"),ellipse(1290,544,540,359,"url(#haze)")]
    for x, y, height in [(973,230,235),(1380,157,315),(1436,174,275),(1670,122,282)]:
        art.extend([path(f"M{x} {y}l-7 {height}h26l-6 {-height}Z","#4b4b4b"),
                    line(x-1,y+7,x+13,y+7,"#8a8a8a",2,.72)])
        for i in range(7):
            art.append(ellipse(x+i*15,y-i*6,16+i*8,8+i*3,"url(#haze)",.13))
    for x in range(570,1900,61):
        w,h=R.randint(29,115),R.randint(138,305)
        value=R.randint(63,94)
        art.append(path(f"M{x} 610V{610-h}h{w*.29:.1f}v-20h{w*.45:.1f}v20h{w*.26:.1f}V610Z",
                        f"rgb({value},{value},{value})"))
        for _ in range(14):
            art.append(rect(x+R.randrange(2,w-3),R.randrange(637-h,591),R.randrange(1,5),R.randrange(4,16),"#c1c1c1",.35))
    river_arch="M480 279L1760 108 1900 158V209L1804 197 1695 226V524H1652V288Q1641 207 1543 252L1376 280Q1289 312 1283 387V592H1230V397Q1214 328 1149 360L943 433 480 470Z"
    art.extend([path(river_arch,"#393939","#737373",1.5),
                path("M490 293L1759 123 1890 172M566 417L1120 341 1770 180","none","#8f8f8f",2,.38),
                path("M663 451L1910 350 1910 376 663 482Z","#303030"),
                path("M663 434L1910 333M663 443L1910 343","none","#232323",2)])
    art.append('<clipPath id="arch">'+path(river_arch,"white")+'</clipPath><g clip-path="url(#arch)">')
    for x in range(520,1910,31):
        y=280-(x-480)*.134
        art.extend((line(x,y,x-5,y+365,"#1c1c1c",1,.35),
                    line(x+4,y+12,x-1,y+365,"#999999",.8,.21)))
    for y in range(284,650,24):
        art.append(line(480,y,1900,y-190,"#949494",.7,.22))
    art.append('</g>')
    for x in range(688,1890,22):
        y=433-(x-663)*.081
        art.append(line(x,y,x,y+41,"#252525",2,.84))
    wall="M0 0H550L869 233 908 620 632 661 0 715Z"
    art.extend([path(wall,"url(#facade)"),
                '<clipPath id="wall">'+path(wall,"white")+'</clipPath><g clip-path="url(#wall)">'])
    for y in range(2,720,23):
        art.extend((line(0,y,919,y-36,"#202020",1.1,.85),line(0,y+2,919,y-34,"#969696",.6,.47)))
        for x in range((y//23%2)*40,919,81):
            off=x*.039
            art.append(line(x,y-off,x,y+22-off,"#181818",1,.55))
    for _ in range(1100):
        x,y=R.uniform(0,910),R.uniform(8,705)
        art.append(line(x,y,x+R.uniform(-2,1),y+R.uniform(2,48),
                        R.choice(("#171717","#929292")),R.uniform(.35,2.4),R.uniform(.08,.32)))
    for x,y,w,h in [(44,174,186,313),(277,202,147,272),(485,239,112,219),(657,291,85,167)]:
        art.extend(glass(x,y,w,h,x in (44,485,657)))
    art.extend([path("M0 551L870 500 875 515 0 568Z","#1b1b1b"),
                path("M0 550L870 499","none","#a3a3a3",1.6,.59),
                path("M457 0V470Q457 503 475 518V664","none","#151515",13),
                path("M463 0V469Q463 497 481 516V657","none","#838383",2,.6),
                '</g>',
                ellipse(1222,570,450,171,"url(#haze)",.74)])
    save("lamp-yard.svg",(0,0,W,715),art,defs)


if __name__ == "__main__":
    side_elevation()
    pavement("track-ground.svg", True)
    steelwork()
    lamp_yard()
    pavement("lamp-ground.svg", False)
